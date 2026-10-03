//! Water geometry: the open-ocean grid, rivers, and lakes.
//!
//! The ocean is a fixed, camera-following grid whose vertex spacing grows
//! exponentially with distance (like the terrain mesh), so Gerstner waves
//! can displace real geometry near the camera while one draw still reaches
//! the horizon. The vertex shader moves the grid with the camera, so it is
//! built once and never re-uploaded.
//!
//! Rivers, lakes and waterfalls come from the terrain's own drainage
//! (`terrain/hydrology.rs`), shaped into the heightmap by the channel
//! stage (`terrain/channels.rs`). Here they become geometry: river ribbons
//! whose vertices carry the current, slope, bend and depth; flat lake and
//! oxbow surfaces; and for each waterfall a curved sheet, mist sprites and
//! a plunge pool, in a buffer of their own.

use vista_types::{RiverOptions, WaterOptions};

use crate::maths::{length2, smoothstep};
use crate::render::terrain_mesh::full_detail_height;
use crate::terrain::biomes::SurfaceSample;
use crate::terrain::channels::{
  channel_depth, condition_channels, height_at, kinoshita, kinoshita_table, raw_streams,
  rock_banks, width_discharge, CarveRecord, ChannelContext, ChannelPoint, Fall, FallStep, Oxbow,
  RawStream, Reach, ReachKind, GRAVITY,
};
use crate::terrain::drainage::NO_RECEIVER;
use crate::terrain::heightmap::HeightMap;
use crate::terrain::hydrology::{build_hydrology, Hydrology, Mouth, NO_LAKE};
use crate::terrain::water_mask::PaintedRiver;

/// CPU mirror of water uniforms used by shaders.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterUniforms {
  /// Sea level in metres.
  pub sea_level_metres: f32,
  /// Wave scale.
  pub wave_scale: f32,
  /// Reflection strength.
  pub reflectivity: f32,
  /// Shoreline blend distance.
  pub shoreline_softness_metres: f32,
}

impl From<&WaterOptions> for WaterUniforms {
  fn from(options: &WaterOptions) -> Self {
    Self {
      sea_level_metres: options.sea_level_metres,
      wave_scale: options.wave_scale,
      reflectivity: options.reflectivity,
      shoreline_softness_metres: options.shoreline_softness_metres,
    }
  }
}

/// Ocean vertex kind (`params[0]`).
pub const WATER_KIND_OCEAN: f32 = 0.0;
/// River vertex kind (`params[0]`).
pub const WATER_KIND_RIVER: f32 = 1.0;
/// Lake vertex kind (`params[0]`).
pub const WATER_KIND_LAKE: f32 = 2.0;
/// Waterfall sheet vertex kind (`params[0]`).
pub const WATER_KIND_FALL: f32 = 3.0;
/// Waterfall mist sprite vertex kind (`params[0]`).
pub const WATER_KIND_SPRAY: f32 = 4.0;
/// Plunge pool vertex kind (`params[0]`).
pub const WATER_KIND_POOL: f32 = 5.0;

/// Most rings, and the segments, in a plunge pool's disc. Rings are about
/// a third of a heightmap sample apart, from two to four of them, so the
/// film follows the ground.
const POOL_RINGS: usize = 4;
const POOL_SEGMENTS: usize = 24;

/// Depth of the film a plunge pool leaves where it spills over ground
/// below its foot.
const POOL_FILM_METRES: f32 = 0.15;

/// One water vertex (56 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WaterVertex {
  /// Ocean: local offset from the camera-snapped grid origin (y unused).
  /// Rivers and lakes: world position in terrain metres.
  pub position: [f32; 3],
  /// Surface current in metres per second (x, z).
  pub flow: [f32; 2],
  /// x: kind (`WATER_KIND_*`; rivers add how far into its step-pool the
  /// vertex lies, 0 to 0.98, see [`WaterVertex::kind`]), y: across-channel
  /// coordinate (-1 to 1) for
  /// rivers and falls, z: local vertex spacing in metres for the ocean
  /// grid, half width on this vertex's side for river ribbons, impact
  /// speed for falls, sprite size for mist, w: distance along the
  /// centreline from the reach's first point for rivers, in metres (0
  /// elsewhere).
  pub params: [f32; 4],
  /// Rivers: slope, curvature (-1 to 1), depth (metres), °C. Lakes:
  /// unused, unused, depth, °C. Falls: metres travelled down the sheet,
  /// sheet length, °C, height. Pools: bowl depth, fall height times
  /// discharge, unused, °C. Mist: fall height, discharge, °C, pool
  /// radius.
  pub extra: [f32; 4],
  /// Rivers: eddy strength, 0 to 1, positive for an eddy on the
  /// `across = 1` bank and negative for one on the `across = -1` bank
  /// (eddies sit on one bank at a time). 0 for all other water.
  pub swirl: f32,
}

impl WaterVertex {
  /// The vertex's kind (`WATER_KIND_*`), without a river's step-pool
  /// fraction.
  pub fn kind(&self) -> f32 {
    self.params[0].floor()
  }
}

/// Build a flat quad covering the terrain footprint at the given sea level.
///
/// Winding is counter-clockwise when viewed from above, matching the
/// renderer's `front_face: Ccw` convention. Retained for simple hosts and
/// tests; the renderer itself draws [`build_ocean_grid`].
pub fn build_water_plane(
  half_width_metres: f32,
  half_height_metres: f32,
  sea_level_metres: f32,
) -> [WaterVertex; 6] {
  let y = sea_level_metres;
  let vertex = |x: f32, z: f32| WaterVertex {
    position: [x, y, z],
    flow: [0.0, 0.0],
    params: [WATER_KIND_OCEAN, 0.0, 0.0, 0.0],
    extra: [0.0; 4],
    swirl: 0.0,
  };
  let a = vertex(-half_width_metres, -half_height_metres);
  let b = vertex(-half_width_metres, half_height_metres);
  let c = vertex(half_width_metres, -half_height_metres);
  let d = vertex(half_width_metres, half_height_metres);

  [a, b, c, b, d, c]
}

/// Grid steps per band before the ocean vertex spacing doubles.
const OCEAN_BAND_WIDTH: i32 = 16;

/// Finest ocean vertex spacing, in metres.
pub const OCEAN_BASE_SPACING_METRES: f32 = 1.5;

/// The ocean grid is recentred in steps of this many metres; must be a
/// multiple of every spacing used inside the displaced region.
pub const OCEAN_SNAP_METRES: f32 = 96.0;

fn ocean_offset(grid_distance: i32, base_spacing: f32, half: i32, far_reach: f32) -> (f32, f32) {
  let sign = grid_distance.signum() as f32;
  let mut remaining = grid_distance.abs();

  if remaining >= half {
    // The outermost ring is pushed out to the horizon.
    return (sign * far_reach, far_reach);
  }

  let mut offset = 0.0;
  let mut step = base_spacing;

  while remaining > 0 {
    let take = remaining.min(OCEAN_BAND_WIDTH);
    offset += take as f32 * step;
    remaining -= take;

    if remaining > 0 {
      step *= 2.0;
    }
  }

  (sign * offset, step)
}

/// Build the camera-following ocean grid. `samples_per_side` must be odd.
///
/// Returns vertices in grid-local metres (the shader adds the snapped
/// camera position) and a triangle list.
pub fn build_ocean_grid(
  samples_per_side: u32,
  far_reach_metres: f32,
) -> (Vec<WaterVertex>, Vec<u32>) {
  let samples = samples_per_side.max(3) | 1;
  let half = (samples / 2) as i32;
  let mut vertices = Vec::with_capacity((samples * samples) as usize);

  for gz in 0..samples as i32 {
    let (z, spacing_z) = ocean_offset(gz - half, OCEAN_BASE_SPACING_METRES, half, far_reach_metres);

    for gx in 0..samples as i32 {
      let (x, spacing_x) =
        ocean_offset(gx - half, OCEAN_BASE_SPACING_METRES, half, far_reach_metres);

      vertices.push(WaterVertex {
        position: [x, 0.0, z],
        flow: [0.0, 0.0],
        params: [WATER_KIND_OCEAN, 0.0, spacing_x.max(spacing_z), 0.0],
        extra: [0.0; 4],
        swirl: 0.0,
      });
    }
  }

  let quads = samples - 1;
  let mut indices = Vec::with_capacity((quads * quads * 6) as usize);

  for gz in 0..quads {
    for gx in 0..quads {
      let a = gz * samples + gx;
      let b = a + 1;
      let c = a + samples;
      let d = c + 1;
      indices.extend_from_slice(&[a, c, b, b, c, d]);
    }
  }

  (vertices, indices)
}

/// Rivers, lakes and waterfalls derived from a heightmap.
#[derive(Clone, Debug, Default)]
pub struct RiverNetwork {
  /// River ribbon, lake and oxbow vertices in world metres.
  pub vertices: Vec<WaterVertex>,
  /// Triangle list indices for `vertices`.
  pub indices: Vec<u32>,
  /// Waterfall sheets and mist, drawn after the rest by their own
  /// pipeline. Plunge pools are water surfaces, in `vertices`.
  pub fall_vertices: Vec<WaterVertex>,
  /// Triangle list indices for `fall_vertices`.
  pub fall_indices: Vec<u32>,
  /// Full-resolution mask of samples under a river channel, lake or
  /// plunge pool.
  pub mask: Vec<bool>,
  /// Original heights of every changed sample, for restoring the terrain.
  pub carved: Vec<(usize, f32)>,
  /// Number of channel reaches.
  pub river_count: u32,
  /// Channel reaches, in heightmap sample coordinates.
  pub reaches: Vec<Reach>,
  /// Waterfalls, in heightmap sample coordinates.
  pub falls: Vec<Fall>,
  /// Lakes, in heightmap sample coordinates.
  pub lakes: Vec<LakeSummary>,
  /// Whether any lake, river or waterfall is below 0 °C.
  pub freezing: bool,
  /// Distance to water, for wet banks and reeds.
  pub wet: WetBanks,
  /// Water entering from beyond the map: where it enters, in heightmap
  /// sample coordinates, its water level and its discharge.
  pub inflows: Vec<InflowPoint>,
  /// Bank strips beside streams narrower than a heightmap sample, drawn
  /// over the terrain by their own pipeline.
  pub bank_vertices: Vec<BankVertex>,
  /// Triangle list indices for `bank_vertices`.
  pub bank_indices: Vec<u32>,
  /// Slow streams narrower than a heightmap sample, as runs of world x, z
  /// and half width along their drawn centrelines, for reeds on their true
  /// banks.
  pub brooks: Vec<Vec<[f32; 3]>>,
  /// Bed materials by river banks: gravel, sand, mud and rock weights (0
  /// to 255, summing to the share they take) for samples the channel stage
  /// changed, one entry per sample. See [`bed_materials`].
  pub bed: Vec<(u32, [u8; 4])>,
  /// Bankside greening per heightmap sample, 0 to 255 for 0 to 1, or
  /// empty when there is none. See [`riparian_field`].
  pub riparian: Vec<u8>,
  /// Every stream as drawn, loops included: runs of world x, z and half
  /// width along its centreline, for keeping trees out of the water.
  pub channels: Vec<Vec<[f32; 3]>>,
  /// The bed stones along each of `channels`, point by point: their
  /// median size in metres and the stone lattice's chance of one (see
  /// `boulders::stone_bed`).
  pub stones: Vec<Vec<[f32; 2]>>,
  /// How densely riparian plants line each of `channels`, point by point
  /// (see [`riparian_band`]).
  pub bands: Vec<Vec<f32>>,
  /// Where rivers meet the sea, for their plumes.
  pub mouths: Vec<SeaMouth>,
  /// Upstream drainage area, from the river hydrology.
  pub drainage: crate::terrain::drainage::DrainageArea,
  /// Mean discharge in cubic metres per second on the same grid as
  /// `drainage`, for map export.
  pub discharge: Vec<f32>,
  /// The signed distance to the drawn water's edge at four times the
  /// heightmap's resolution, near channels, for wet margins and beds.
  pub field: crate::render::channel_field::ChannelField,
}

/// One bank vertex (44 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BankVertex {
  /// World position in terrain metres.
  pub position: [f32; 3],
  /// Unit direction away from the water (x, z).
  pub outward: [f32; 2],
  /// x: where across the bank's profile ([`BANK_PROFILE`] order: 0 at
  /// the water's edge, 1 the face's foot, 2 its top under the lip, 3 the
  /// lip's edge, 4 the back of the turf), y: metres along the centreline,
  /// as the ribbon's `along`, z: flow speed in metres per second, w: the
  /// side, as the ribbon's `across` (-1 or 1).
  pub params: [f32; 4],
  /// x: how much of a cut bank this is (0 an inner bank's shelf, 1 a cut
  /// face), y: the stream's width in metres.
  pub shape: [f32; 2],
}

/// The points across a bank's profile, from the water outwards: the
/// water's edge, the wet margin's outer edge at the foot of the face, the
/// top of the face under the turf lip, the lip's edge and the back of the
/// turf on the ground.
pub const BANK_PROFILE: usize = 5;

/// Where a river meets the sea: its plume tints the sea beyond.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeaMouth {
  /// World x and z of the mouth's middle.
  pub at: [f32; 2],
  /// Unit direction (x, z) the river flows out to sea in.
  pub toward: [f32; 2],
  /// Width at the mouth in metres.
  pub width: f32,
  /// Mean discharge in cubic metres per second.
  pub discharge: f32,
  /// The catchment's colour (see [`catchment_code`]).
  pub code: f32,
}

/// How far a river's plume reaches out to sea, in metres: 5 widths for a
/// brook of 1 m³/s, rising to 20 for a river of 1000 m³/s.
pub fn plume_length(width: f32, discharge: f32) -> f32 {
  width * (5.0 + 15.0 * (discharge.max(1.0).log10() / 3.0).min(1.0))
}

/// How much of a river's plume tints the sea `along` metres seawards of
/// its mouth and `across` to one side: from the mouth (`width` metres
/// across) it spreads to four times as wide and fades to nothing by
/// `length`. `plume_share` in `water.wgsl`.
pub fn plume_share(along: f32, across: f32, width: f32, length: f32) -> f32 {
  let spread = 0.5 * width * (1.0 + 3.0 * (along / length).clamp(0.0, 1.0));
  smoothstep((along + 0.5 * width) / (0.5 * width))
    * (1.0 - smoothstep((along - 0.4 * length) / (0.6 * length)))
    * (1.0 - smoothstep((across.abs() - 0.5 * spread) / (0.5 * spread)))
}

/// The eight mouths whose plumes matter most from `camera` (world x, z):
/// the largest rivers, the nearer the better. Two vectors each for the
/// frame (`FrameUniforms::mouths`): x, z and the way out to sea, then
/// width, plume length and the catchment's colour; a width of 0 where
/// there is none.
pub fn plume_mouths(mouths: &[SeaMouth], camera: [f32; 2]) -> [[f32; 4]; 16] {
  let mut best: [(f32, usize); 8] = [(0.0, usize::MAX); 8];

  for (index, mouth) in mouths.iter().enumerate() {
    let distance = length2(mouth.at[0] - camera[0], mouth.at[1] - camera[1]);
    let score = mouth.discharge.max(0.01) / (1.0 + distance / 2000.0);

    if let Some(slot) = best.iter().position(|(other, _)| score > *other) {
      best.copy_within(slot..7, slot + 1);
      best[slot] = (score, index);
    }
  }

  let mut words = [[0.0; 4]; 16];

  for (k, (_, index)) in best.iter().enumerate() {
    if let Some(mouth) = mouths.get(*index) {
      words[2 * k] = [mouth.at[0], mouth.at[1], mouth.toward[0], mouth.toward[1]];
      words[2 * k + 1] = [
        mouth.width,
        plume_length(mouth.width, mouth.discharge),
        mouth.code,
        0.0,
      ];
    }
  }

  words
}

/// An inflow in use.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InflowPoint {
  /// Heightmap sample coordinates.
  pub position: [f32; 2],
  /// Water level, in metres.
  pub level: f32,
  /// Mean discharge in cubic metres per second.
  pub discharge: f32,
}

/// Distance to the nearest water, for wet banks, bankside grass and reeds,
/// at the resolution of the height texture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WetBanks {
  /// Field width in texels.
  pub width: u32,
  /// Field height in texels.
  pub height: u32,
  /// Heightmap samples per texel along each axis.
  pub stride: u32,
  /// Distance to the nearest river, lake or waterfall edge, 0 to
  /// [`WET_BANK_RANGE_METRES`] as 0 to 255.
  pub distance: Vec<u8>,
  /// The same, to still or slow water only: lakes, oxbows and rivers
  /// slower than [`REED_SPEED`].
  pub still: Vec<u8>,
}

/// The distance the wet-bank field reaches, in metres.
pub const WET_BANK_RANGE_METRES: f32 = 40.0;

/// Reeds grow by rivers slower than this, in metres per second.
pub const REED_SPEED: f32 = 0.6;

/// Largest wet-bank field, in texels per side: the height texture's size.
const WET_BANK_MAX: u32 = 2048;

impl WetBanks {
  /// Build the field from full-resolution masks of water and of still
  /// water. The water's edge lies half a sample beyond its last sample.
  pub fn build(map: &HeightMap, water: &[bool], still: &[bool]) -> Self {
    let edge = -0.5 * map.metadata.metres_per_sample.max(0.001);
    let seeds: Vec<f32> = water
      .iter()
      .map(|wet| if *wet { edge } else { f32::MAX })
      .collect();
    Self::from_seeds(map, &seeds, still)
  }

  /// Build the field from `water`, each sample's distance in metres to
  /// the water's edge where it is known (negative inside the water, and
  /// `f32::MAX` elsewhere), and a full-resolution mask of still water.
  pub fn from_seeds(map: &HeightMap, water: &[f32], still: &[bool]) -> Self {
    let map_width = map.metadata.width;
    let map_height = map.metadata.height;

    if map_width < 2 || map_height < 2 || !water.iter().any(|d| *d < f32::MAX) {
      return Self::default();
    }

    let stride = (map_width.max(map_height).saturating_sub(1) / (WET_BANK_MAX - 1)).max(1);
    let width = (map_width - 1) / stride + 1;
    let height = (map_height - 1) / stride + 1;
    let step = map.metadata.metres_per_sample.max(0.001) * stride as f32;
    let field = |seed: &dyn Fn(usize) -> f32| {
      let mut distance = Vec::with_capacity((width * height) as usize);

      for y in 0..height {
        for x in 0..width {
          distance.push(seed(((y * stride) * map_width + x * stride) as usize));
        }
      }

      crate::terrain::biomes::chamfer_distance(
        width as usize,
        height as usize,
        step,
        &mut distance,
      );
      distance
        .iter()
        .map(|d| {
          (d.max(0.0) / WET_BANK_RANGE_METRES * 255.0)
            .round()
            .min(255.0) as u8
        })
        .collect()
    };
    let edge = -0.5 * step;

    Self {
      width,
      height,
      stride,
      distance: field(&|index| water[index]),
      still: field(&|index| if still[index] { edge } else { f32::MAX }),
    }
  }

  /// Distances in metres to any water and to still water at a heightmap
  /// sample, or the full range where there is no field.
  pub fn at(&self, x: u32, y: u32) -> (f32, f32) {
    if self.distance.is_empty() {
      return (WET_BANK_RANGE_METRES, WET_BANK_RANGE_METRES);
    }

    let tx = (x / self.stride).min(self.width - 1);
    let ty = (y / self.stride).min(self.height - 1);
    let index = (ty * self.width + tx) as usize;
    let metres = |value: u8| value as f32 / 255.0 * WET_BANK_RANGE_METRES;
    (metres(self.distance[index]), metres(self.still[index]))
  }
}

/// Seeds for the wet-bank field (see [`WetBanks::from_seeds`]). Lakes,
/// plunge pools and rivers at least a sample wide fill their samples, so
/// their edge lies half a sample out. A stream narrower than a sample
/// covers only part of the samples it marks, so those samples, and their
/// neighbours, take the distance to its drawn edge: the wet margin and
/// the grass beside it follow the stream's true width, not the grid's.
fn wet_bank_seeds(
  map: &HeightMap,
  water: &[bool],
  lakes: &[bool],
  drawn: &[Vec<ChannelPoint>],
  falls: &[Fall],
) -> Vec<f32> {
  let (width, height) = (map.metadata.width as i32, map.metadata.height as i32);
  let metres = map.metadata.metres_per_sample.max(0.001);
  let edge = -0.5 * metres;
  let mut full = lakes.to_vec();
  let mut mark = |x: f32, y: f32, radius: f32| {
    let r = radius.ceil() as i32;
    let (cx, cy) = (x.round() as i32, y.round() as i32);

    for sy in (cy - r).max(0)..=(cy + r).min(height - 1) {
      for sx in (cx - r).max(0)..=(cx + r).min(width - 1) {
        full[(sy * width + sx) as usize] = true;
      }
    }
  };

  for points in drawn {
    for p in points.iter().filter(|p| p.width >= metres || p.falling) {
      mark(p.x, p.y, 0.65 * p.width / metres + 0.5);
    }
  }

  for fall in falls {
    mark(fall.foot[0], fall.foot[1], fall.pool_radius / metres + 0.5);

    for step in &fall.steps {
      mark(step.foot[0], step.foot[1], step.pool_radius / metres + 0.5);
    }
  }

  let mut seeds: Vec<f32> = water
    .iter()
    .zip(&full)
    .map(|(wet, full)| if *wet && *full { edge } else { f32::MAX })
    .collect();

  for points in drawn {
    for pair in points.windows(2) {
      let (a, b) = (&pair[0], &pair[1]);

      if a.width >= metres || b.width >= metres || a.falling || b.falling {
        continue;
      }

      // The samples around the segment; the field carries the distance
      // on from them.
      let (x0, x1) = (a.x.min(b.x).floor() as i32, a.x.max(b.x).ceil() as i32);
      let (y0, y1) = (a.y.min(b.y).floor() as i32, a.y.max(b.y).ceil() as i32);
      let segment = [b.x - a.x, b.y - a.y];
      let length = (segment[0] * segment[0] + segment[1] * segment[1]).max(1e-12);
      let half = 0.25 * (a.width + b.width);

      for sy in y0.max(0)..=y1.min(height - 1) {
        for sx in x0.max(0)..=x1.min(width - 1) {
          let (px, py) = (sx as f32 - a.x, sy as f32 - a.y);
          let t = ((px * segment[0] + py * segment[1]) / length).clamp(0.0, 1.0);
          let (dx, dy) = (px - segment[0] * t, py - segment[1] * t);
          let slot = &mut seeds[(sy * width + sx) as usize];
          let reach = (*slot + half) / metres;

          // Only a nearer edge than the one found needs the square root.
          if reach > 0.0 && dx * dx + dy * dy < reach * reach {
            *slot = length2(dx, dy) * metres - half;
          }
        }
      }
    }
  }

  seeds
}

/// What the rest of the engine needs to know about a lake.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LakeSummary {
  /// Water surface in metres.
  pub surface: f32,
  /// Mean annual temperature at the outlet, in °C.
  pub celsius: f32,
  /// Whether the lake has no outlet.
  pub endorheic: bool,
  /// Shore points in heightmap sample coordinates, about one per 32 m.
  pub shore: Vec<[f32; 2]>,
}

/// Which water is left to build: natural drainage, and painted water.
pub struct RiverSources<'a> {
  /// Surface samples before rivers were carved (may be empty).
  pub surface: &'a [SurfaceSample],
  /// Seeds springs, meanders and deltas.
  pub seed: u64,
  /// Painted rivers from a water mask, already oriented downhill.
  pub painted: Vec<PaintedRiver>,
  /// Samples a painted mask already changed, with their original heights.
  pub record: CarveRecord,
}

/// Lakes freeze below this mean temperature, in °C, fully at -2 °C.
pub const LAKE_FREEZE_CELSIUS: f32 = 0.0;
/// Rivers freeze below this mean temperature, in °C, fully at -7 °C.
pub const RIVER_FREEZE_CELSIUS: f32 = -5.0;
/// Waterfalls turn to ice below this mean temperature, in °C.
pub const FALL_FREEZE_CELSIUS: f32 = -8.0;

/// How much of a water surface is frozen, 0 (open) to 1 (solid), where
/// freezing starts at `start` °C and is complete 2 °C colder. This is the
/// CPU twin of `freeze_fraction` in `water.wgsl`.
pub fn freeze_fraction(celsius: f32, start: f32) -> f32 {
  ((start - celsius) / 2.0).clamp(0.0, 1.0)
}

/// Extract rivers, lakes and waterfalls from `map`, shaping their channels
/// into it. The original height of every changed sample is recorded in
/// [`RiverNetwork::carved`] so the caller can restore the terrain later.
pub fn build_river_network(
  map: &mut HeightMap,
  options: &RiverOptions,
  sources: RiverSources<'_>,
) -> RiverNetwork {
  let map_width = map.metadata.width;
  let map_height = map.metadata.height;
  let RiverSources {
    surface,
    seed,
    painted,
    mut record,
  } = sources;
  let mut network = RiverNetwork {
    mask: vec![false; map.heights.len()],
    ..RiverNetwork::default()
  };

  // The narrowest channel is 0.6 m wide and deep, so a map a few metres
  // across has no room for one.
  let span = map.metadata.metres_per_sample * (map_width.min(map_height) as f32 - 1.0);

  if map_width < 8
    || map_height < 8
    || span < MIN_RIVER_MAP_METRES
    || (!options.enabled && painted.is_empty() && record.is_empty())
  {
    network.carved = record.into_original();
    return network;
  }

  let mut hydrology = build_hydrology(map, surface, options, seed);
  network.drainage = crate::terrain::drainage::DrainageArea {
    width: hydrology.width,
    height: hydrology.height,
    stride: hydrology.stride,
    cells: std::mem::take(&mut hydrology.area),
  };
  network.inflows = hydrology
    .inflows
    .iter()
    .map(|inflow| {
      let (x, y) = hydrology.sample_xy(inflow.cell);
      InflowPoint {
        position: [x as f32, y as f32],
        level: hydrology.filled[inflow.cell as usize],
        discharge: inflow.discharge,
      }
    })
    .collect();
  let mut painted_cells = vec![false; hydrology.ground.len()];
  let painted: Vec<RawStream> = painted
    .iter()
    .filter_map(|river| painted_stream(&hydrology, map, river, options, &mut painted_cells))
    .collect();
  let mut streams = if options.enabled {
    raw_streams(&hydrology)
  } else {
    Vec::new()
  };

  // Painted rivers win over the drainage: natural streams end where they
  // reach painted water, and the painted rivers are cut first so the
  // streams joining them meet them at their level.
  streams.retain_mut(|stream| {
    let cut = stream
      .points
      .iter()
      .position(|p| painted_cells[cell_at(&hydrology, *p) as usize]);

    match cut {
      Some(0) => false,
      Some(i) => {
        stream.points.truncate(i + 1);
        stream.levels.truncate(i + 1);
        stream.discharge.truncate(i + 1);
        stream.mouth = Mouth::Join;
        true
      }
      None => true,
    }
  });
  streams.splice(0..0, painted);
  let channels = condition_channels(
    map,
    &hydrology,
    streams,
    &ChannelContext {
      surface,
      options,
      seed,
    },
    &mut record,
  );
  let metres = map.metadata.metres_per_sample.max(0.001);
  let half = [
    (map_width as f32 - 1.0) * metres * 0.5,
    (map_height as f32 - 1.0) * metres * 0.5,
  ];
  let current = options.current_speed.max(0.0);

  // Where reaches meet, and lake shores: ribbon points there are never
  // dropped, so joins stay connected.
  let mut ends = vec![false; map.heights.len()];

  for point in channels
    .reaches
    .iter()
    .flat_map(|reach| [reach.points.first(), reach.points.last()])
    .flatten()
  {
    mark_sample(&mut ends, map, [point.x, point.y]);
  }

  // Every sample next to an end, found once: ribbons ask this of each of
  // their points, and loops multiply the points.
  let mut near_end = vec![false; ends.len()];

  for (index, _) in ends.iter().enumerate().filter(|(_, end)| **end) {
    let (x, y) = (
      (index as u32 % map_width) as i32,
      (index as u32 / map_width) as i32,
    );

    for ny in (y - 1).max(0)..=(y + 1).min(map_height as i32 - 1) {
      for nx in (x - 1).max(0)..=(x + 1).min(map_width as i32 - 1) {
        near_end[(ny as u32 * map_width + nx as u32) as usize] = true;
      }
    }
  }

  let anchored = |point: &ChannelPoint| {
    // Drawn points stay within `CORRIDOR_SAMPLES` of the carved path, so
    // they round to a sample on the map; the clamp only guards the index.
    let x = (point.x.round() as i32).clamp(0, map_width as i32 - 1) as u32;
    let y = (point.y.round() as i32).clamp(0, map_height as i32 - 1) as u32;
    let beside_end = near_end[(y * map_width + x) as usize];
    beside_end || hydrology.lake[cell_at(&hydrology, [point.x, point.y]) as usize] != NO_LAKE
  };

  let table = kinoshita_table();
  // Sub-sample loops multiply a stream's points. On wide, gentle country
  // full of small streams they would run to millions, so the map shares a
  // budget of two points for every sample of channel, main stems first.
  let mut loop_budget = 2
    * channels
      .reaches
      .iter()
      .flat_map(|reach| reach.points.windows(2))
      .map(|pair| length2(pair[1].x - pair[0].x, pair[1].y - pair[0].y))
      .sum::<f32>() as usize
    + 100_000;

  // Every reach's drawn centreline first, so the shared buffers are sized
  // once: loops give small streams hundreds of thousands of points, and
  // doubling those buffers as they filled cost more than filling them.
  // The loop budget goes to the highest Strahler orders first (then to
  // main stems, which come first).
  let order_of = |reach: &Reach| reach.points.iter().map(|p| p.order).max().unwrap_or(0);
  let mut by_order: Vec<usize> = channels
    .reaches
    .iter()
    .enumerate()
    .map(|(index, reach)| ((255 - order_of(reach) as usize) << 24) | index)
    .collect();
  by_order.sort_unstable();
  let mut drawn: Vec<Vec<ChannelPoint>> = vec![Vec::new(); channels.reaches.len()];
  let mut steps: Vec<Vec<f32>> = vec![Vec::new(); channels.reaches.len()];
  let stones = crate::render::lattice::lattice_seed(seed) ^ crate::render::boulders::STONE_SALT;

  for key in by_order {
    let index = key & 0xff_ffff;
    let reach = &channels.reaches[index];
    // A braided belt shows only through its threads.
    if reach.kind == ReachKind::Belt {
      continue;
    }

    let phase = crate::maths::hash_u64(seed ^ index as u64) as f32 / u64::MAX as f32;
    let centre = sub_sample_centreline(
      &reach.points,
      metres,
      options.meanders,
      phase,
      &table,
      &anchored,
      &mut loop_budget,
    );
    drawn[index] = ribbon_points(&centre, metres, &anchored);
    settle_on_ground(&mut drawn[index], map);
    steps[index] = step_pools(&mut drawn[index], map, (metres, half), stones, &anchored);
  }

  let arcs: Vec<Vec<f32>> = drawn
    .iter()
    .map(|points| {
      crate::terrain::channels::arc_lengths(
        &points.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
        metres,
      )
    })
    .collect();
  let mut strips: Vec<BankStrips> = drawn
    .iter()
    .zip(&arcs)
    .enumerate()
    .map(|(index, (points, arcs))| plan_bank_strips(map, points, arcs, metres, seed ^ index as u64))
    .collect();
  let rows: usize = drawn.iter().map(Vec::len).sum();
  network.vertices.reserve(rows * 2);
  network.indices.reserve(rows * 6);

  network.channels = drawn
    .iter()
    .filter(|points| points.len() > 1)
    .map(|points| {
      points
        .iter()
        .map(|p| {
          let world = to_world([p.x, p.y], metres, half);
          [world[0], world[1], 0.5 * p.width]
        })
        .collect()
    })
    .collect();
  network.stones = drawn
    .iter()
    .filter(|points| points.len() > 1)
    .map(|points| {
      points
        .iter()
        .map(|p| {
          if p.falling {
            [0.0; 2]
          } else {
            crate::render::boulders::stone_bed(p.speed, p.depth, p.slope, p.width)
          }
        })
        .collect()
    })
    .collect();

  // A braided belt's bars carry only pioneer scrub.
  network.bands = drawn
    .iter()
    .zip(&channels.reaches)
    .filter(|(points, _)| points.len() > 1)
    .map(|(points, reach)| {
      let pioneer = if reach.kind == ReachKind::Thread {
        0.4
      } else {
        1.0
      };
      points
        .iter()
        .map(|p| pioneer * riparian_band(p, options.riparian))
        .collect()
    })
    .collect();

  // Eddies below bends and falls, and on the main stem's bank beside each
  // join, on the tributary's side, for 2 w of the tributary downstream.
  // There, too, the tributary's banks take the main stem's.
  let mut swirl: Vec<Vec<f32>> = drawn
    .iter()
    .zip(&arcs)
    .map(|(points, s)| swirls(points, s))
    .collect();
  let map_width = map.metadata.width;
  let sample = |p: &ChannelPoint| {
    (p.y.round().clamp(0.0, (map.metadata.height - 1) as f32) as u32 * map_width
      + p.x.round().clamp(0.0, (map_width - 1) as f32) as u32) as usize
  };
  let mut first = vec![u32::MAX; map.heights.len()];

  for (index, points) in drawn.iter().enumerate() {
    for p in points {
      let at = sample(p);
      first[at] = first[at].min(index as u32);
    }
  }

  for (index, points) in drawn.iter().enumerate() {
    let (Some(end), Some(before)) = (
      points.last(),
      points.len().checked_sub(2).map(|k| &points[k]),
    ) else {
      continue;
    };
    let main = first[sample(end)] as usize;

    if main >= index {
      continue;
    }

    let stem = &drawn[main];
    let nearest = |p: &&ChannelPoint| length2(p.x - end.x, p.y - end.y);
    let Some(j) =
      (0..stem.len()).min_by(|a, b| nearest(&&stem[*a]).total_cmp(&nearest(&&stem[*b])))
    else {
      continue;
    };
    let next = &stem[(j + 1).min(stem.len() - 1)];
    let here = &stem[j.saturating_sub(1)];
    let side = ((next.x - here.x) * (before.y - stem[j].y)
      - (next.y - here.y) * (before.x - stem[j].x))
      .signum();
    eddy(&mut swirl[main], &arcs[main], j, 2.0 * end.width, side);
    let entered = strips[main]
      .rows
      .iter()
      .filter(|row| row.side == side)
      .min_by_key(|row| (row.point as i64 - j as i64).unsigned_abs())
      .map(|row| row.shape);

    if let Some(shape) = entered {
      blend_join(&mut strips[index], &arcs[index], end.width, shape);
    }
  }

  fit_bank_strips(&mut strips, &drawn, map);
  let banks: usize = strips.iter().map(|s| s.vertices(s.stride)).sum();
  // Into an empty buffer, `reserve` allocates exactly this much.
  network.bank_vertices.reserve(banks);
  network.bank_indices.reserve(banks * 6);

  for ((points, strips), arcs) in drawn.iter().zip(&strips).zip(&arcs) {
    add_bank_strips(&mut network, map, points, (strips, arcs), metres, half);
  }

  // Each river carries its catchment's colour on its swirl.
  let shares = catchment_shares(&hydrology, &network.drainage.cells, surface, map_width);

  for (points, swirl) in drawn.iter().zip(&mut swirl) {
    for (p, swirl) in points.iter().zip(swirl) {
      *swirl += 4.0 * catchment_code(shares[cell_at(&hydrology, [p.x, p.y]) as usize]);
    }
  }

  // Mouths at the sea, where the ground beyond lies under it.
  let sea = map.metadata.sea_level_metres;

  for (points, reach) in drawn.iter().zip(&channels.reaches) {
    let n = points.len();

    if reach.kind == ReachKind::Belt || n < 2 {
      continue;
    }

    let (p, q) = (&points[n - 1], &points[n - 2]);
    let length = length2(p.x - q.x, p.y - q.y).max(1e-6);
    let toward = [(p.x - q.x) / length, (p.y - q.y) / length];
    let out = p.width / metres;

    if p.level > sea + 0.5
      || full_detail_height(map, p.x + toward[0] * out, p.y + toward[1] * out) >= sea
    {
      continue;
    }

    network.mouths.push(SeaMouth {
      at: to_world([p.x, p.y], metres, half),
      toward,
      width: p.width,
      discharge: p.discharge,
      code: catchment_code(shares[cell_at(&hydrology, [p.x, p.y]) as usize]),
    });
  }

  for ((points, swirl), steps) in drawn.iter().zip(&swirl).zip(&steps) {
    push_ribbon(&mut network, (points, swirl, steps), metres, half, current);
    add_brooks(&mut network, points, metres, half);
  }

  for oxbow in &channels.oxbows {
    add_oxbow(&mut network, oxbow, metres, half);
  }

  add_lakes(&mut network, &hydrology, map, half);

  for fall in &channels.falls {
    add_fall(&mut network, map, fall, metres, half, seed);
  }

  // Still water, for reeds: lakes and oxbows, and slow rivers.
  let mut still = network.mask.clone();

  for oxbow in &channels.oxbows {
    for point in &oxbow.points {
      mark_sample(&mut still, map, *point);
    }
  }

  // Brooks narrower than a sample grow their reeds along their true banks
  // (see `RiverNetwork::brooks`), so only wider rivers count here.
  for reach in &channels.reaches {
    for point in reach
      .points
      .iter()
      .filter(|point| point.speed < REED_SPEED && point.width >= metres)
    {
      mark_sample(&mut still, map, [point.x, point.y]);
    }
  }

  // Before the channels join the mask, it holds only still water.
  network.riparian = riparian_field(map, &channels.reaches, &network.mask, options.riparian);
  let lakes = network.mask.clone();

  for (slot, value) in network.mask.iter_mut().zip(&channels.mask) {
    *slot |= *value;
  }

  let seeds = wet_bank_seeds(map, &network.mask, &lakes, &drawn, &channels.falls);
  network.wet = WetBanks::from_seeds(map, &seeds, &still);
  network.field = crate::render::channel_field::ChannelField::build(map, &drawn, &lakes);

  network.freezing = network
    .lakes
    .iter()
    .any(|lake| lake.celsius < LAKE_FREEZE_CELSIUS)
    || channels
      .reaches
      .iter()
      .any(|reach| reach.points.iter().any(|point| point.celsius < 0.0))
    || channels.falls.iter().any(|fall| fall.celsius < 0.0);
  network.river_count = channels.reaches.len() as u32;
  network.reaches = channels.reaches;
  network.falls = channels.falls;
  network.carved = record.into_original();
  network.discharge = std::mem::take(&mut hydrology.discharge);
  network.bed = bed_materials(map, &network.reaches, &network.carved, &network.mask);
  crate::terrain::heightmap::update_stats(&map.heights, &map.no_data, &mut map.metadata);
  network
}

/// Gravel, sand and mud along rivers at least 0.75 samples wide, sorted
/// by the flow: gravel at 1 m/s and over, sand from 0.4 to 1 m/s, and
/// mud below, which the wet banks darken. They cover a band beside the
/// water and, on the inner side of bends, point bars reaching out to
/// 1.5 w; mouths near sea level are sand. Only samples the channel stage
/// touched (`carved` or `mask`), from the water surface up to the bank
/// top, and only as weights blended with the ground that is there.
/// Narrower streams get their bed look from the bank strips. Rock walls
/// ([`rock_banks`]) take over the banks of powerful, steep reaches of any
/// width, up the wall to w or a sample above the water. One entry
/// per sample, from the reach that claims the largest share of it.
pub fn bed_materials(
  map: &HeightMap,
  reaches: &[Reach],
  carved: &[(usize, f32)],
  mask: &[bool],
) -> Vec<(u32, [u8; 4])> {
  let mut touched = mask.to_vec();

  for (index, _) in carved {
    touched[*index] = true;
  }

  // The largest share of each sample: a first pass finds it, and the
  // second keeps the first stamp that reaches it.
  let mut best = vec![0u8; touched.len()];
  let mut bed = Vec::new();

  for keep in [false, true] {
    stamp_bed(map, reaches, &touched, &mut |index, share, weights| {
      if !keep {
        best[index] = best[index].max(share);
      } else if share == best[index] {
        best[index] = 0;
        bed.push((
          index as u32,
          weights.map(|weight| (weight * f32::from(share)).round() as u8),
        ));
      }
    });
  }

  bed
}

/// Call `out(index, share, [gravel, sand, mud, rock])` for every touched sample
/// each wide river segment covers, as [`bed_materials`] describes; the
/// share is 1 to 255.
fn stamp_bed(
  map: &HeightMap,
  reaches: &[Reach],
  touched: &[bool],
  out: &mut dyn FnMut(usize, u8, [f32; 4]),
) {
  let metres = map.metadata.metres_per_sample;
  let (width, height) = (map.metadata.width as i32, map.metadata.height as i32);
  let sea = map.metadata.sea_level_metres;

  let heights = &map.heights;

  for (pair, belt) in reaches.iter().flat_map(|reach| {
    reach
      .points
      .windows(2)
      .map(move |pair| (pair, reach.kind == ReachKind::Belt))
  }) {
    let (a, b) = (pair[0], pair[1]);
    let w = 0.5 * (a.width + b.width);
    let rock = rock_banks(&a).max(rock_banks(&b));
    // Beds of narrower streams come from the bank strips; rock walls
    // come to streams of any width.
    let bed = a.width.min(b.width) >= 0.75 * metres;

    if !(bed || rock > 0.0) || a.falling || b.falling {
      continue;
    }

    let band = metres.max(0.5 * w);
    // Rock covers the band 1.5 samples out, so a diagonal reach does not
    // leave beads of rock on alternate samples.
    let rock_band = (1.5 * metres).max(w) * f32::from(u8::from(rock > 0.0));
    let reach = (0.5 * w + band.max(1.5 * w).max(rock_band)) / metres;
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length = (dx * dx + dy * dy).max(1e-6);
    let x0 = ((a.x.min(b.x) - reach).floor() as i32).max(0);
    let x1 = ((a.x.max(b.x) + reach).ceil() as i32).min(width - 1);
    let y0 = ((a.y.min(b.y) - reach).floor() as i32).max(0);
    let y1 = ((a.y.max(b.y) + reach).ceil() as i32).min(height - 1);

    for y in y0..=y1 {
      for x in x0..=x1 {
        let index = (y * width + x) as usize;

        if !touched[index] {
          continue;
        }

        let (px, py) = (x as f32 - a.x, y as f32 - a.y);
        let t = ((px * dx + py * dy) / length).clamp(0.0, 1.0);
        let lerp = |u: f32, v: f32| u + (v - u) * t;
        let edge = length2(px - dx * t, py - dy * t) * metres - 0.5 * lerp(a.width, b.width);
        let curvature = lerp(a.curvature, b.curvature);
        let inner = (dx * py - dy * px) * curvature > 0.0;
        let bar = 1.5 * w * smoothstep(curvature.abs() / 0.2) * f32::from(u8::from(inner));
        let level = lerp(a.level, b.level);
        let ground = map.heights[index];
        let above = smoothstep((ground - level + 0.1) / 0.1);
        // Loose beds end at the bank top; rock walls climb above it.
        let below_top = 1.0 - smoothstep((ground - level - lerp(a.depth, b.depth).max(1.0)) / 0.5);
        let loose = (1.0 - smoothstep((edge - 0.5 * band) / (0.5 * band)))
          .max(1.0 - smoothstep((edge - 0.7 * bar) / (0.3 * bar).max(1e-3)))
          * below_top
          * f32::from(u8::from(bed))
          * (1.0 - rock);
        // The bank a bend cuts is bare earth and rock where it stands
        // steeper than 35 degrees.
        let fall = |dx: i32, dy: i32| {
          let (nx, ny) = ((x + dx).clamp(0, width - 1), (y + dy).clamp(0, height - 1));
          heights[(ny * width + nx) as usize]
        };
        let steepness =
          length2(fall(1, 0) - fall(-1, 0), fall(0, 1) - fall(0, -1)) / (2.0 * metres);
        let cut = 0.6
          * smoothstep(curvature.abs() / 0.2)
          * smoothstep((steepness - 0.7) / 0.3)
          * f32::from(u8::from(bed && !inner && edge > -0.25 * w && edge < w));
        let rock = (rock
          * (1.0 - smoothstep((edge - 0.5 * rock_band) / (0.5 * rock_band).max(1e-3))))
        .max(cut);
        let keep = loose.max(rock);
        let share = (keep * above * 255.0).round() as u8;

        if share >= 5 {
          let speed = lerp(a.speed, b.speed);
          let mouth = 1.0 - smoothstep((level - sea) / 1.5);
          // A braided belt's bars are gravel.
          let gravel =
            (smoothstep((speed - 0.9) / 0.2) * (1.0 - mouth)).max(f32::from(u8::from(belt)));
          let mud = (1.0 - smoothstep((speed - 0.3) / 0.2)) * (1.0 - mouth) * (1.0 - gravel);
          let total = loose + rock;
          let (loose, rock) = (loose / total, rock / total);
          out(
            index,
            share,
            [
              gravel * loose,
              (1.0 - gravel - mud) * loose,
              mud * loose,
              rock,
            ],
          );
        }
      }
    }
  }
}

/// Bankside greening: `(1 - d / R)^2 x strength` (at most 1) at a
/// distance `d` from water, where `R = clamp(25 + 12 sqrt(Q), 25, 400)`
/// metres for rivers and 40 m for lakes, oxbows and pools (`still`). A
/// two-pass chamfer carries the largest remaining reach `R - d` outwards
/// with its source's `R`. Empty when `strength` is 0 or there is no water.
pub fn riparian_field(
  map: &HeightMap,
  reaches: &[Reach],
  still: &[bool],
  strength: f32,
) -> Vec<u8> {
  let (width, height) = (map.metadata.width as usize, map.metadata.height as usize);
  let metres = map.metadata.metres_per_sample;

  if strength <= 0.0 || (reaches.is_empty() && !still.contains(&true)) {
    return Vec::new();
  }

  let mut left: Vec<f32> = still
    .iter()
    .map(|wet| if *wet { 40.0 } else { 0.0 })
    .collect();
  let mut radius: Vec<u16> = left.iter().map(|reach| *reach as u16).collect();

  for point in reaches.iter().flat_map(|reach| &reach.points) {
    let (x, y) = (point.x.round() as usize, point.y.round() as usize);

    if x < width && y < height {
      let reach = (25.0 + 12.0 * point.discharge.max(0.0).sqrt()).min(400.0);
      let index = y * width + x;

      if reach > left[index] {
        left[index] = reach;
        radius[index] = reach as u16;
      }
    }
  }

  let diagonal = metres * std::f32::consts::SQRT_2;
  let mut reach_from = |to: usize, from: usize, cost: f32| {
    if left[from] - cost > left[to] {
      left[to] = left[from] - cost;
      radius[to] = radius[from];
    }
  };

  for y in 0..height {
    for x in 0..width {
      let index = y * width + x;

      if x > 0 {
        reach_from(index, index - 1, metres);
      }

      if y > 0 {
        reach_from(index, index - width, metres);

        if x > 0 {
          reach_from(index, index - width - 1, diagonal);
        }

        if x + 1 < width {
          reach_from(index, index - width + 1, diagonal);
        }
      }
    }
  }

  for y in (0..height).rev() {
    for x in (0..width).rev() {
      let index = y * width + x;

      if x + 1 < width {
        reach_from(index, index + 1, metres);
      }

      if y + 1 < height {
        reach_from(index, index + width, metres);

        if x + 1 < width {
          reach_from(index, index + width + 1, diagonal);
        }

        if x > 0 {
          reach_from(index, index + width - 1, diagonal);
        }
      }
    }
  }

  left
    .iter()
    .zip(&radius)
    .map(|(left, radius)| {
      let near = (left / f32::from((*radius).max(1))).max(0.0);
      ((near * near * strength).min(1.0) * 255.0).round() as u8
    })
    .collect()
}

/// The flow cell nearest a heightmap sample position.
fn cell_at(hydrology: &Hydrology, point: [f32; 2]) -> u32 {
  let stride = hydrology.stride as f32;
  let x = ((point[0] / stride).round() as u32).min(hydrology.width - 1);
  let y = ((point[1] / stride).round() as u32).min(hydrology.height - 1);
  y * hydrology.width + x
}

/// A painted river as a stream: its discharge comes from the drainage,
/// and below its end it follows the drainage on until it meets a natural
/// channel, a lake, the sea or the map edge, so it joins the network. A
/// painted tributary ends where it joins its painted river.
fn painted_stream(
  hydrology: &Hydrology,
  map: &HeightMap,
  river: &PaintedRiver,
  options: &RiverOptions,
  painted_cells: &mut [bool],
) -> Option<RawStream> {
  let mut points = river.points.clone();
  let mut levels: Vec<f32> = points.iter().map(|p| height_at(map, p[0], p[1])).collect();
  // It carries at least what a natural river of its width would, so its
  // depth, speed and colour follow the same rules.
  let least = width_discharge(river.width, options.width_scale.clamp(0.1, 10.0));
  let mut discharge: Vec<f32> = points
    .iter()
    .map(|p| hydrology.discharge[cell_at(hydrology, *p) as usize].max(least))
    .collect();
  let order = |p: [f32; 2]| hydrology.strahler[cell_at(hydrology, p) as usize].max(1);
  let mut orders: Vec<u8> = points.iter().map(|p| order(*p)).collect();

  for point in &points {
    painted_cells[cell_at(hydrology, *point) as usize] = true;
  }

  // A painted tributary ends on the painted river it joins.
  if river.joins {
    return Some(RawStream {
      points,
      levels,
      discharge,
      orders,
      min_width: river.width,
      mouth: Mouth::Join,
      painted: true,
    });
  }

  let mut cell = cell_at(hydrology, *points.last()?);
  // At most a step a cell: only a receiver cycle could walk further.
  let mut steps = 0;
  let mouth = loop {
    let r = hydrology.receiver[cell as usize];
    steps += 1;

    if r == NO_RECEIVER || steps > hydrology.receiver.len() {
      debug_assert!(r == NO_RECEIVER, "the receivers form a cycle");
      break Mouth::Edge;
    }

    let (x, y) = hydrology.sample_xy(r);
    let lake = hydrology.lake[r as usize];
    let mouth = if lake != NO_LAKE {
      Some(Mouth::Lake(lake))
    } else if hydrology.is_sea(r) {
      Some(Mouth::Sea)
    } else if hydrology.is_drawn(r) && !painted_cells[r as usize] {
      Some(Mouth::Join)
    } else {
      None
    };
    points.push([x as f32, y as f32]);
    levels.push(match mouth {
      Some(Mouth::Sea) => hydrology.sea,
      Some(Mouth::Lake(id)) => hydrology.lakes[id as usize].surface,
      _ => hydrology.filled[r as usize],
    });
    // The cell it ends on carries its main stem's (or the sea's) water
    // and order, not its own.
    let last = (*discharge.last()?, *orders.last()?);
    discharge.push(if mouth.is_some() {
      last.0
    } else {
      hydrology.discharge[r as usize].max(last.0)
    });
    orders.push(if mouth.is_some() {
      last.1
    } else {
      hydrology.strahler[r as usize].max(last.1)
    });

    if let Some(mouth) = mouth {
      break mouth;
    }

    painted_cells[r as usize] = true;
    cell = r;
  };

  Some(RawStream {
    points,
    levels,
    discharge,
    orders,
    min_width: river.width,
    mouth,
    painted: true,
  })
}

/// The narrowest a map may be, in metres, to hold rivers.
pub const MIN_RIVER_MAP_METRES: f32 = 50.0;

/// Restore every sample changed by [`build_river_network`].
pub fn restore_carving(map: &mut HeightMap, carved: &[(usize, f32)]) {
  for (index, height) in carved {
    if let Some(slot) = map.heights.get_mut(*index) {
      *slot = *height;
    }
  }

  crate::terrain::heightmap::update_stats(&map.heights, &map.no_data, &mut map.metadata);
}

fn mark_sample(mask: &mut [bool], map: &HeightMap, point: [f32; 2]) {
  let x = (point[0].round().max(0.0) as u32).min(map.metadata.width - 1);
  let y = (point[1].round().max(0.0) as u32).min(map.metadata.height - 1);
  mask[(y * map.metadata.width + x) as usize] = true;
}

fn to_world(point: [f32; 2], metres: f32, half: [f32; 2]) -> [f32; 2] {
  [point[0] * metres - half[0], point[1] * metres - half[1]]
}

/// One row across a strip.
struct StripRow {
  centre: [f32; 2],
  /// Unit vector towards the `across = 1` side.
  side: [f32; 2],
  /// Half widths on the `across = -1` and `across = 1` sides.
  half: [f32; 2],
  level: f32,
  flow: [f32; 2],
  extra: [f32; 4],
  /// Distance along the centreline in metres (see `WaterVertex::params`).
  along: f32,
  /// See `WaterVertex::swirl`.
  swirl: f32,
  /// How far into its step-pool the row lies (see [`step_pools`]), 0
  /// off the steps.
  step: f32,
}

/// Push one ribbon strip, a row at a time; `skip(i)` leaves out the quads
/// after row `i`.
fn push_strip(
  vertices: &mut Vec<WaterVertex>,
  indices: &mut Vec<u32>,
  kind: f32,
  rows: &mut dyn ExactSizeIterator<Item = StripRow>,
  skip: &dyn Fn(usize) -> bool,
) {
  let first = vertices.len() as u32;
  let count = rows.len();
  vertices.reserve(count * 2);
  indices.reserve(count.saturating_sub(1) * 6);

  for row in rows {
    for (across, half) in [-1.0f32, 1.0].into_iter().zip(row.half) {
      vertices.push(WaterVertex {
        position: [
          row.centre[0] + row.side[0] * half * across,
          row.level,
          row.centre[1] + row.side[1] * half * across,
        ],
        flow: row.flow,
        params: [kind + row.step, across, half, row.along],
        extra: row.extra,
        swirl: row.swirl,
      });
    }
  }

  for i in 0..count.saturating_sub(1) as u32 {
    if skip(i as usize) {
      continue;
    }

    let a = first + i * 2;
    indices.extend_from_slice(&[a, a + 1, a + 2, a + 2, a + 1, a + 3]);
  }
}

/// The points a river ribbon is drawn through. Steep reaches are
/// subdivided so no segment is longer than half the width (or half a
/// sample: the ground has no finer detail to follow), then straight,
/// uniform runs are thinned out (see [`simplify_ribbon`]).
fn ribbon_points(
  points: &[ChannelPoint],
  metres: f32,
  anchored: &dyn Fn(&ChannelPoint) -> bool,
) -> Vec<ChannelPoint> {
  if points.len() < 2 {
    return points.to_vec();
  }

  let pieces = |a: &ChannelPoint, b: &ChannelPoint| {
    let length = length2(b.x - a.x, b.y - a.y) * metres;
    let limit = if a.slope.max(b.slope) > 0.02 {
      (0.5 * a.width.min(b.width)).max(0.5 * metres)
    } else {
      metres
    };
    (length / limit).ceil().clamp(1.0, 64.0) as usize
  };
  // Sized up front: loops give a stream many points, and growing these
  // point by point cost more than filling them.
  let total = 1
    + points
      .windows(2)
      .map(|pair| pieces(&pair[0], &pair[1]))
      .sum::<usize>();
  let mut dense: Vec<ChannelPoint> = Vec::with_capacity(total);
  dense.push(points[0]);
  // Which dense points are the channel's own, not subdivisions.
  let mut original = Vec::with_capacity(total);
  original.push(true);

  for pair in points.windows(2) {
    let (a, b) = (pair[0], pair[1]);
    let pieces = pieces(&a, &b);

    for k in 1..=pieces {
      let t = k as f32 / pieces as f32;
      let mut p = a;
      p.x = a.x + (b.x - a.x) * t;
      p.y = a.y + (b.y - a.y) * t;
      p.level = a.level + (b.level - a.level) * t;
      p.width = a.width + (b.width - a.width) * t;
      p.depth = a.depth + (b.depth - a.depth) * t;
      p.slope = a.slope + (b.slope - a.slope) * t;
      p.speed = a.speed + (b.speed - a.speed) * t;
      p.curvature = a.curvature + (b.curvature - a.curvature) * t;
      p.celsius = a.celsius + (b.celsius - a.celsius) * t;
      p.falling = if k == pieces {
        b.falling
      } else {
        a.falling && b.falling
      };
      dense.push(p);
      original.push(k == pieces);
    }
  }

  simplify_ribbon(&dense, &original, metres, anchored)
}

/// Lower a drawn stream's water onto the ground under it: at most its
/// depth and 0.5 m above the carved ground at each point, and never rising
/// downstream. Levels follow the samples the stream was routed through,
/// but its drawn line runs between them; across a steep face, half a
/// sample to the side can be metres lower, and the water would hang in
/// the air above it.
fn settle_on_ground(points: &mut [ChannelPoint], map: &HeightMap) {
  let mut previous = f32::INFINITY;

  for p in points {
    let ground = full_detail_height(map, p.x, p.y);
    p.level = p.level.min(ground + p.depth + 0.5).min(previous);
    previous = p.level;
  }
}

/// Each flow cell's catchment: the share of the ground draining through
/// it (`area` cells, as the hydrology gathers it) that is peat (x: bog, or
/// ground wetter than 80 % and colder than 8 °C), and that is glacier ice
/// (y).
fn catchment_shares(
  hydrology: &Hydrology,
  area: &[f32],
  surface: &[SurfaceSample],
  map_width: u32,
) -> Vec<[f32; 2]> {
  use crate::terrain::drainage::accumulate;
  let cells = hydrology.receiver.len();
  let peaty = |cell: usize| {
    surface
      .get(hydrology.sample_index(cell as u32, map_width))
      .is_some_and(|s| {
        s.biome == vista_types::BiomeKind::SwampWetlands as u8
          || (s.moisture >= 204 && s.celsius() < 8.0)
      })
  };
  let flow = |weights: Vec<f32>| accumulate(&hydrology.order, &hydrology.receiver, weights);
  let peat = flow(
    (0..cells)
      .map(|cell| f32::from(u8::from(peaty(cell))))
      .collect(),
  );
  let ice = flow(
    hydrology
      .glacier
      .iter()
      .map(|ice| f32::from(u8::from(*ice)))
      .collect(),
  );
  (0..cells)
    .map(|cell| {
      [
        peat[cell] / area[cell].max(1.0),
        ice[cell] / area[cell].max(1.0),
      ]
    })
    .collect()
}

/// A river's catchment colour as one whole number, which rides on its
/// swirl as multiples of 4 (`water.wgsl` reads it back): peat from 0 to 7
/// as its catchment goes from 30 to 70 % bog, plus 8 times glacial flour
/// from 0 to 7 as it goes from no ice to a fifth ice.
pub fn catchment_code([peat, ice]: [f32; 2]) -> f32 {
  let peat = smoothstep((peat - 0.3) / 0.4);
  let flour = (ice * 5.0).min(1.0);
  (peat * 7.0).round() + 8.0 * (flour * 7.0).round()
}

/// How densely riparian plants grow beside a stream at `p`, for a
/// `riparian` option of 0 to 2 (0 none, 1 as nature has it, 2 twice as
/// dense): none on falls, thinned where unit stream power passes
/// 300 W/m² and scours the banks bare, and thickened by slow water.
pub fn riparian_band(p: &ChannelPoint, riparian: f32) -> f32 {
  if p.falling {
    return 0.0;
  }

  let power = 9810.0 * p.speed * p.depth * p.slope;
  let scour = 1.0 - 0.7 * smoothstep((power - 300.0) / 300.0);
  let slow = 1.0 + 0.5 * (1.0 - smoothstep((p.speed - 0.3) / 0.7));
  riparian.clamp(0.0, 2.0) * scour * slow
}

/// Step-pools form on slopes from 6 % to 30 %: gentler streams run in
/// riffles and pools, steeper ones tumble down in a cascade of white water
/// (the shader's rapids) with no pools between their steps.
const STEP_SLOPES: [f32; 2] = [0.06, 0.3];

/// Streams narrower than this, in metres, are the colluvial headwaters
/// above the step-pools, and steps dropping less than [`STEP_DROP`] would
/// not show: neither steps. (Without them a map of rills 0.4 m wide would
/// carry a step every metre, seven times the ribbon's rows.)
const STEP_MIN_WIDTH: f32 = 0.75;
/// See [`STEP_MIN_WIDTH`]: the least drop of a step, in metres.
const STEP_DROP: f32 = 0.1;

/// How long a step's face of falling water is, in metres.
const STEP_FACE_METRES: f32 = 0.2;

/// The spacing of step-pools on a slope `slope` in a stream `width` wide:
/// Judd's `L = 0.31 S^-1.19` metres (Judd, 1964), kept within 1.5 to 3
/// widths (Chin, 1999).
pub fn step_spacing(slope: f32, width: f32) -> f32 {
  (0.31 * slope.max(1e-3).powf(-1.19)).clamp(1.5 * width, 3.0 * width)
}

/// Water that steps: on slopes within [`STEP_SLOPES`], a drawn stream
/// wider than [`STEP_MIN_WIDTH`] whose steps would drop at least
/// [`STEP_DROP`] drops from pool to pool instead of sliding down an even
/// ramp. Its steps form behind the largest of its own stones (the stream
/// stones' lattice, `stones` its seed, as `boulders::stone_candidate`
/// places them): one in each stretch [`step_spacing`] long, give or take a
/// quarter. Each pool is level, at the smooth level at its lip (its
/// lowest), but at most `d + 0.5` above the ground anywhere under it and
/// never above the pool before it: the water steps down below the smooth
/// surface, never above it, so it never stands proud of its banks. Nor
/// does it sink to less than 40 % of its depth over the ground: where a
/// step would fall further, its pool tilts there. At each lip a row is added, and another
/// [`STEP_FACE_METRES`] below it, so the drop is a short, steep face of
/// water. Steps stop two widths short of joins, lake shores and falls
/// (`anchored` and falling points). Returns, per point, how far into its
/// pool it lies, 0.02 just below a lip to 0.98 at the next lip, or 0 off
/// the steps: the water shader whitens the water below each lip by it.
fn step_pools(
  points: &mut Vec<ChannelPoint>,
  map: &HeightMap,
  (metres, half): (f32, [f32; 2]),
  stones: u32,
  anchored: &dyn Fn(&ChannelPoint) -> bool,
) -> Vec<f32> {
  let n = points.len();
  let mut into = vec![0.0; n];

  let stepping = |p: &ChannelPoint| {
    p.slope > STEP_SLOPES[0]
      && p.slope <= STEP_SLOPES[1]
      && p.width >= STEP_MIN_WIDTH
      && step_spacing(p.slope, p.width) * p.slope >= STEP_DROP
  };

  if n < 3 || !points.iter().any(stepping) {
    return into;
  }

  let s = crate::terrain::channels::arc_lengths(
    &points.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
    metres,
  );
  // Points two widths from anything a step must not come near: each such
  // point opens a stretch, counted in and out along the stream.
  let mut opens = vec![0i32; n + 1];

  for (i, p) in points.iter().enumerate() {
    if p.falling || i == 0 || i == n - 1 || anchored(p) {
      opens[s.partition_point(|&along| along < s[i] - 2.0 * p.width)] += 1;
      opens[s.partition_point(|&along| along <= s[i] + 2.0 * p.width)] -= 1;
    }
  }

  let mut open = 0;
  let blocked: Vec<bool> = opens[..n]
    .iter()
    .map(|change| {
      open += change;
      open > 0
    })
    .collect();

  let steep = |k: usize| !blocked[k] && stepping(&points[k]);
  // Where along the stream each step's lip lies, and its run's ends.
  let mut lips: Vec<f32> = Vec::new();
  let mut runs: Vec<(usize, usize)> = Vec::new();
  let mut i = 0;

  while i < n {
    if !steep(i) {
      i += 1;
      continue;
    }

    let start = i;

    while i < n && steep(i) {
      i += 1;
    }

    let end = i - 1;
    let mut cursor = s[start];
    let mut any = false;

    // Each lip lies past the last, so a run of n points holds far fewer
    // than 4 n of them; the bound only stops a run whose spacing is not
    // a number.
    for _ in 0..4 * n {
      let k = s.partition_point(|&along| along < cursor).min(end);
      let (slope, width) = (points[k].slope, points[k].width);
      let spacing = step_spacing(slope, width);
      let window = [
        cursor + (0.75 * spacing).max(1.5 * width),
        cursor + (1.25 * spacing).min(3.0 * width),
      ];

      if window[1] > s[end] - 0.5 * spacing {
        break;
      }

      let lip = largest_stone(points, &s, window, (metres, half), stones)
        .unwrap_or(0.5 * (window[0] + window[1]));
      lips.push(lip);
      cursor = lip;
      any = true;
    }

    if any {
      runs.push((start, end));
    }
  }

  if lips.is_empty() {
    return into;
  }

  // Add a row at each lip and one just below it.
  let mut stepped: Vec<ChannelPoint> = Vec::with_capacity(n + 2 * lips.len());
  let mut along: Vec<f32> = Vec::with_capacity(n + 2 * lips.len());
  let mut cuts = lips
    .iter()
    .flat_map(|lip| [*lip, lip + STEP_FACE_METRES])
    .peekable();
  let lerp_point = |a: &ChannelPoint, b: &ChannelPoint, t: f32| {
    let lerp = |u: f32, v: f32| u + (v - u) * t;
    ChannelPoint {
      x: lerp(a.x, b.x),
      y: lerp(a.y, b.y),
      level: lerp(a.level, b.level),
      bed: lerp(a.bed, b.bed),
      width: lerp(a.width, b.width),
      depth: lerp(a.depth, b.depth),
      discharge: lerp(a.discharge, b.discharge),
      slope: lerp(a.slope, b.slope),
      speed: lerp(a.speed, b.speed),
      curvature: lerp(a.curvature, b.curvature),
      celsius: lerp(a.celsius, b.celsius),
      ..*a
    }
  };

  for k in 0..n {
    while let Some(&cut) = cuts.peek() {
      if k == 0 || cut >= s[k] {
        break;
      }

      if cut > s[k - 1] {
        let t = (cut - s[k - 1]) / (s[k] - s[k - 1]).max(1e-6);
        stepped.push(lerp_point(&points[k - 1], &points[k], t));
        along.push(cut);
      }

      cuts.next();
    }

    stepped.push(points[k]);
    along.push(s[k]);
  }

  // A face is one quad: the points that fell within it go.
  let mut face = 0;
  let inside: Vec<bool> = along
    .iter()
    .map(|&at| {
      while face < lips.len() && at >= lips[face] + STEP_FACE_METRES - 1e-4 {
        face += 1;
      }

      face < lips.len() && at > lips[face] + 1e-4
    })
    .collect();
  let mut gone = inside.iter();
  stepped.retain(|_| gone.next() != Some(&true));
  let mut gone = inside.iter();
  along.retain(|_| gone.next() != Some(&true));

  // Each pool from the row below one lip (or its run's start) to the next
  // lip (or its run's end) is level.
  into = vec![0.0; stepped.len()];
  let mut lip_index = 0;

  for (start, end) in runs {
    let (first, last) = (s[start], s[end]);
    let mut bounds = vec![first];

    while lip_index < lips.len() && lips[lip_index] < last {
      bounds.extend([lips[lip_index], lips[lip_index] + STEP_FACE_METRES]);
      lip_index += 1;
    }

    bounds.push(last);
    let mut previous = f32::INFINITY;

    for pool in bounds.chunks_exact(2) {
      let (from, to) = (pool[0], pool[1]);
      let members =
        along.partition_point(|&at| at < from - 1e-4)..along.partition_point(|&at| at <= to + 1e-4);
      // The smooth level only falls, so its lowest is at the lip.
      let mut level = stepped[members.end - 1].level.min(previous);

      for p in &stepped[members.clone()] {
        level = level.min(full_detail_height(map, p.x, p.y) + p.depth + 0.5);
      }

      for k in members {
        // The drawn ground cannot dig out a pool, so the water never sinks
        // into it: it stays 40 % of its depth over the ground, the pool
        // tilting where a step falls further than that.
        let p = &stepped[k];
        let floor = full_detail_height(map, p.x, p.y) + 0.4 * p.depth;
        stepped[k].level = level.max(floor.min(p.level));
        into[k] = 0.02 + 0.96 * ((along[k] - from) / (to - from).max(1e-6)).clamp(0.0, 1.0);
      }

      previous = level;
    }
  }

  // The water downstream of the steps never stands above the last pool.
  let mut previous = f32::INFINITY;

  for p in &mut stepped {
    p.level = p.level.min(previous);
    previous = p.level;
  }

  *points = stepped;
  into
}

/// Where along a stream (points `points`, `s` metres down it) the largest
/// of its own stones in its water between `window[0]` and `window[1]`
/// lies, if any: the stream stones' lattice (seed `stones`) as
/// `boulders::stone_candidate` places them, with each point's own bed.
fn largest_stone(
  points: &[ChannelPoint],
  s: &[f32],
  window: [f32; 2],
  (metres, half): (f32, [f32; 2]),
  stones: u32,
) -> Option<f32> {
  use crate::render::lattice::{jittered, point_hash, unit, TREE_TRAITS_SALT};
  let first = s
    .partition_point(|&along| along < window[0])
    .saturating_sub(1);
  let last = s
    .partition_point(|&along| along <= window[1])
    .min(points.len() - 1);
  let world: Vec<[f32; 2]> = points[first..=last]
    .iter()
    .map(|p| to_world([p.x, p.y], metres, half))
    .collect();
  let reach = points[first..=last]
    .iter()
    .fold(0.0f32, |w, p| w.max(p.width));
  let (mut low, mut high) = ([f32::MAX; 2], [f32::MIN; 2]);

  for p in &world {
    low = [low[0].min(p[0]), low[1].min(p[1])];
    high = [high[0].max(p[0]), high[1].max(p[1])];
  }

  let mut best: Option<(f32, f32)> = None;

  for iz in (low[1] - reach).floor() as i32..=(high[1] + reach).ceil() as i32 {
    for ix in (low[0] - reach).floor() as i32..=(high[0] + reach).ceil() as i32 {
      let hash = point_hash(ix, iz, stones);
      let at = jittered(ix, iz, hash, crate::render::boulders::STONE_PITCH);
      // The nearest point of the centreline, and how far along it lies.
      let (mut nearest, mut along, mut k_near) = (f32::MAX, 0.0, first);

      for (k, pair) in world.windows(2).enumerate() {
        let d = [pair[1][0] - pair[0][0], pair[1][1] - pair[0][1]];
        let length = (d[0] * d[0] + d[1] * d[1]).max(1e-12);
        let t =
          (((at[0] - pair[0][0]) * d[0] + (at[1] - pair[0][1]) * d[1]) / length).clamp(0.0, 1.0);
        let off = length2(pair[0][0] + d[0] * t - at[0], pair[0][1] + d[1] * t - at[1]);

        if off < nearest {
          nearest = off;
          along = s[first + k] + (s[first + k + 1] - s[first + k]) * t;
          k_near = first + k;
        }
      }

      let p = &points[k_near];
      let [median, chance] = crate::render::boulders::stone_bed(p.speed, p.depth, p.slope, p.width);

      if nearest > 0.5 * p.width
        || unit(hash[2]) >= chance
        || along < window[0]
        || along > window[1]
      {
        continue;
      }

      let size = crate::render::boulders::stone_size(
        unit(point_hash(ix, iz, stones ^ TREE_TRAITS_SALT)[0]),
        median,
      );

      if best.is_none_or(|(largest, _)| size > largest) {
        best = Some((size, along));
      }
    }
  }

  best.map(|(_, along)| along)
}

/// A river ribbon through `dense` (from [`ribbon_points`]), 1.3 w wide,
/// so its edge lies on the bank, where the shader fades it out by depth.
fn push_ribbon(
  network: &mut RiverNetwork,
  (dense, swirl, steps): (&[ChannelPoint], &[f32], &[f32]),
  metres: f32,
  half: [f32; 2],
  current: f32,
) {
  let n = dense.len();

  if n < 2 {
    return;
  }

  let world = |i: usize| to_world([dense[i].x, dense[i].y], metres, half);
  let mut along = 0.0;
  let rows = (0..n).map(|i| {
    let p = &dense[i];
    let (prev, here, next) = (
      world(i.saturating_sub(1)),
      world(i),
      world((i + 1).min(n - 1)),
    );
    let tangent = [next[0] - prev[0], next[1] - prev[1]];
    let length = length2(tangent[0], tangent[1]).max(1e-4);
    let tangent = [tangent[0] / length, tangent[1] / length];
    along += length2(here[0] - prev[0], here[1] - prev[1]) * f32::from(u8::from(i > 0));
    let half_width = 0.65 * p.width;
    // On the inner side of a bend the edge would fold back over itself
    // past the centre of curvature, so it stays within 0.9 of the radius.
    let turn =
      (here[0] - prev[0]) * (next[1] - here[1]) - (here[1] - prev[1]) * (next[0] - here[0]);
    let inner = 0.9 * crate::terrain::centreline::radius(prev, here, next);
    let half_widths = if turn > 0.0 {
      [half_width, half_width.min(inner)]
    } else {
      [half_width.min(inner), half_width]
    };
    // The flow always carries the direction, which the shader needs to
    // widen far ribbons across it, even with the current stopped.
    let speed = (p.speed * current).max(0.001);
    StripRow {
      centre: here,
      side: [-tangent[1], tangent[0]],
      half: half_widths,
      level: p.level,
      flow: [tangent[0] * speed, tangent[1] * speed],
      extra: [p.slope, p.curvature, p.depth, p.celsius],
      along,
      swirl: swirl[i],
      step: steps.get(i).copied().unwrap_or(0.0),
    }
  });
  // Falls are drawn as sheets of their own, so the ribbon leaves out its
  // quads over them, and the rows inside a fall, with no quad either side,
  // would be vertices nothing draws. Every row is still built: its
  // tangent and distance along come from its neighbours.
  let falling = |i: usize| dense[i.min(n - 1)].falling;
  let drawn = |i: usize| i == 0 || !(falling(i - 1) && falling(i) && falling(i + 1));
  let kept: Vec<usize> = (0..n).filter(|&i| drawn(i)).collect();
  let rows: Vec<StripRow> = rows
    .enumerate()
    .filter(|(i, _)| drawn(*i))
    .map(|(_, row)| row)
    .collect();
  push_strip(
    &mut network.vertices,
    &mut network.indices,
    WATER_KIND_RIVER,
    &mut rows.into_iter(),
    &|j| falling(kept[j]) && falling(kept[(j + 1).min(kept.len() - 1)]),
  );
}

/// Eddy strength at each point of a drawn reach (see `WaterVertex::swirl`):
/// on the inner bank for 3 w below each bend apex where `|κ| w > 0.3`
/// (curvature over 0.6), rising and falling smoothly, where the flow
/// separates after the apex; and on one bank for 2 w below the foot of
/// every fall, around its pool's outlet.
fn swirls(points: &[ChannelPoint], s: &[f32]) -> Vec<f32> {
  let n = points.len();
  let mut swirl = vec![0.0f32; n];

  for i in 1..n.saturating_sub(1) {
    let c = points[i].curvature;
    let apex = c.abs() > 0.6
      && c.abs() >= points[i - 1].curvature.abs()
      && c.abs() > points[i + 1].curvature.abs();
    let foot = points[i - 1].falling && !points[i].falling;

    if apex || foot {
      let (reach, side) = if apex {
        (3.0 * points[i].width, c.signum())
      } else {
        (2.0 * points[i].width, if i % 2 == 0 { 1.0 } else { -1.0 })
      };
      eddy(&mut swirl, s, i, reach, side);
    }
  }

  swirl
}

/// Add an eddy on `side`'s bank from point `from` for `reach` metres
/// downstream, rising and falling as `sin`; the stronger eddy wins.
fn eddy(swirl: &mut [f32], s: &[f32], from: usize, reach: f32, side: f32) {
  for k in from..swirl.len() {
    let d = s[k] - s[from];

    if d > reach {
      break;
    }

    let value = side * (std::f32::consts::PI * d / reach.max(1e-3)).sin();

    if value.abs() > swirl[k].abs() {
      swirl[k] = value;
    }
  }
}

/// Loops of the sub-sample centreline stay this far, in heightmap
/// samples, from the carved path, inside the carved trench.
pub const CORRIDOR_SAMPLES: f32 = 0.45;

/// The centreline a stream is drawn along. Streams too narrow for their
/// meanders to migrate on the heightmap (11 w under 3 samples, see
/// [`crate::terrain::meander::representable`]) on slopes under 1 %
/// meander at their own wavelength, which the grid cannot carve: a Kinoshita
/// curve of amplitude up to 2.5 w. Steeper ones do not meander, but no
/// mountain stream runs straight: it follows the hollows of its slope,
/// and boulders, roots and banks turn it this way and that. So it wanders
/// in long bends about 3.5 samples (at least 30 widths) long, by up to
/// 0.6 of the corridor, and short ones about 5 widths long, by up to half
/// a width, whose lengths drift. Both are offset along
/// the smooth centreline's normals, kept within [`CORRIDOR_SAMPLES`] of
/// the carved centreline and pinned at its ends, at joins (`joined`),
/// falls and wider water. The hydrology, carving and sounds keep the
/// carved centreline.
fn sub_sample_centreline(
  points: &[ChannelPoint],
  metres: f32,
  strength: f32,
  phase: f32,
  table: &[f32; 64],
  joined: &dyn Fn(&ChannelPoint) -> bool,
  budget: &mut usize,
) -> Vec<ChannelPoint> {
  let strength = strength.clamp(0.0, 1.0);
  // Loops shorter than a sixth of a sample would need many points for
  // little to see.
  let narrow = |p: &ChannelPoint| {
    let narrow = !crate::terrain::meander::representable(p.width, metres)
      && 11.0 * p.width >= metres / 6.0
      && !p.falling;
    f32::from(u8::from(narrow))
  };
  let gentle = |p: &ChannelPoint| 1.0 - smoothstep((p.slope - 0.006) / 0.004);
  let corridor = CORRIDOR_SAMPLES * metres;
  let loops = |p: &ChannelPoint| narrow(p) * gentle(p) * (2.5 * p.width * strength).min(corridor);
  let wander = |p: &ChannelPoint| {
    narrow(p)
      * (1.0 - gentle(p))
      * strength
      * (0.6 * corridor + (0.5 * p.width).min(0.4 * corridor))
  };
  let amplitude = |p: &ChannelPoint| loops(p).max(wander(p));

  if points.len() < 3 || strength <= 0.0 || points.iter().all(|p| amplitude(p) < 0.02 * metres) {
    return points.to_vec();
  }

  // Loops need six points per wavelength, and the wander three per short
  // bend. A reach whose loops would overrun the map's share keeps its
  // carved path.
  let pieces = |a: &ChannelPoint, b: &ChannelPoint| {
    if amplitude(a).max(amplitude(b)) > 0.0 {
      let length = length2(b.x - a.x, b.y - a.y) * metres;
      let step = if wander(a).max(wander(b)) > 0.0 {
        5.0 / 3.0
      } else {
        11.0 / 6.0
      };
      (length / (step * a.width.min(b.width))).clamp(1.0, 256.0) as usize
    } else {
      1
    }
  };
  let counts: Vec<usize> = points
    .windows(2)
    .map(|pair| pieces(&pair[0], &pair[1]))
    .collect();
  let count: usize = counts.iter().sum();

  if count > *budget {
    return points.to_vec();
  }

  *budget -= count;

  let mut out = Vec::with_capacity(count + 1);
  out.push(points[0]);
  let start = phase;
  let mut phase = phase;
  let mut wander_phase = phase * 1.7;
  let mut long_phase = phase * 0.9;
  let mut along = 0.0;
  // Distances along the carved path to the points the loops must pass
  // through: both ends and every join with another reach.
  let mut pins = vec![0.0];
  let mut walked = 0.0;

  for (i, pair) in points.windows(2).enumerate() {
    walked += length2(pair[1].x - pair[0].x, pair[1].y - pair[0].y) * metres;

    if i + 2 == points.len() || joined(&pair[1]) {
      pins.push(walked);
    }
  }

  let mut pin_index = 0;
  // Each point's normal from its neighbours either side: the centreline is
  // smooth, so the loops turn with it instead of flipping at each point.
  let n = points.len();
  let normals: Vec<[f32; 2]> = (0..n)
    .map(|i| {
      let (a, b) = (&points[i.saturating_sub(1)], &points[(i + 1).min(n - 1)]);
      let length = length2(b.x - a.x, b.y - a.y).max(1e-6);
      [-(b.y - a.y) / length, (b.x - a.x) / length]
    })
    .collect();

  for ((i, pair), &pieces) in points.windows(2).enumerate().zip(&counts) {
    let (a, b) = (pair[0], pair[1]);
    let length = length2(b.x - a.x, b.y - a.y) * metres;
    let ends = loops(&a).min(loops(&b));
    let wander_ends = wander(&a).min(wander(&b));

    for k in 1..=pieces {
      let t = k as f32 / pieces as f32;
      let lerp = |u: f32, v: f32| u + (v - u) * t;
      let normal = {
        let (na, nb) = (normals[i], normals[i + 1]);
        let blended = [lerp(na[0], nb[0]), lerp(na[1], nb[1])];
        let size = length2(blended[0], blended[1]).max(1e-6);
        [blended[0] / size, blended[1] / size]
      };
      let mut p = a;
      p.x = lerp(a.x, b.x);
      p.y = lerp(a.y, b.y);
      p.level = lerp(a.level, b.level);
      p.bed = lerp(a.bed, b.bed);
      p.width = lerp(a.width, b.width);
      p.depth = lerp(a.depth, b.depth);
      p.slope = lerp(a.slope, b.slope);
      p.speed = lerp(a.speed, b.speed);
      p.celsius = lerp(a.celsius, b.celsius);
      p.falling = if k == pieces {
        b.falling
      } else {
        a.falling && b.falling
      };
      let step = length / pieces as f32;
      along += step;
      // Real loops vary: the wavelength drifts by up to 30 % and the
      // amplitude between 60 and 100 %, over periods no multiple of it,
      // so a straight carved path does not show a regular wave.
      let drift = (along / (47.0 * p.width).max(0.01) + start * 3.7).sin();
      let swell = 0.8 + 0.2 * (along / (71.0 * p.width).max(0.01) + start * 5.3).cos();
      phase += step / (11.0 * p.width * (1.0 + 0.3 * drift)).max(0.01);
      wander_phase += step / (5.0 * p.width * (1.0 + 0.4 * drift)).max(0.01);
      long_phase += step / ((3.5 * metres).max(30.0 * p.width) * (1.0 + 0.3 * swell)).max(0.01);

      while pin_index + 2 < pins.len() && pins[pin_index + 1] <= along {
        pin_index += 1;
      }

      // Pinned over half a wavelength around each pin, so joins meet.
      let reach = 5.5 * p.width;
      let pin = smoothstep((along - pins[pin_index]) / reach)
        * smoothstep((pins[pin_index + 1] - along) / reach);
      // Long bends by 0.6 of the corridor, short ones by half a width, as
      // shares of the wander's whole amplitude.
      let long = 0.6 * corridor;
      let short = (0.5 * p.width).min(0.4 * corridor);
      let bends = (long * (long_phase * std::f32::consts::TAU).sin()
        + short * (wander_phase * std::f32::consts::TAU).sin())
        / (long + short).max(1e-6);
      let offset = loops(&p).min(ends) * pin * swell * kinoshita(table, phase)
        + wander(&p).min(wander_ends) * pin * bends;
      p.x += normal[0] * offset / metres;
      p.y += normal[1] * offset / metres;
      out.push(p);
    }
  }

  out
}

/// Most bank vertices one map may have. Banks are built once, so the
/// budget is spent when they are built: see [`plan_bank_strips`] and
/// [`fit_bank_strips`].
pub const BANK_STRIP_BUDGET: usize = 300_000;

/// Consecutive bank rows merge where the stream's heading turns by less
/// than this, in degrees.
const STRIP_MERGE_DEGREES: f32 = 4.0;

/// The width of turf a bank's mesh carries beyond the lip, in metres:
/// just enough to meet the ground, whose own grass and tufts take over.
const TURF_METRES: f32 = 0.25;

/// The shape of a bank's profile (see [`BANK_PROFILE`]), in metres.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BankShape {
  /// How much of a cut bank this is: 0 an inner bank's gentle shelf, 1 a
  /// near-vertical face under an overhanging lip.
  pub cut: f32,
  /// The wet margin's width at the water's level.
  pub margin: f32,
  /// The face's height above the water, to the lip.
  pub face: f32,
  /// How far the face's top lies back from its foot.
  pub run: f32,
  /// How far the turf lip hangs out over the face.
  pub overhang: f32,
  /// The lip's thickness, its dark underside.
  pub lip: f32,
}

impl BankShape {
  fn mix(self, other: Self, t: f32) -> Self {
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    Self {
      cut: lerp(self.cut, other.cut),
      margin: lerp(self.margin, other.margin),
      face: lerp(self.face, other.face),
      run: lerp(self.run, other.run),
      overhang: lerp(self.overhang, other.overhang),
      lip: lerp(self.lip, other.lip),
    }
  }

  /// Offsets from the water's edge and heights above the water of the
  /// profile's points up to the lip's edge: [`BANK_PROFILE`] order, less
  /// the turf's back, which lies on the ground.
  pub fn points(&self) -> [[f32; 2]; 4] {
    let foot = self.margin;
    let top = foot + self.run;
    [
      [-0.05, -0.03],
      [foot, 0.03],
      [top, self.face - self.lip],
      [top - self.overhang, self.face],
    ]
  }
}

/// The shape of a stream's bank at a point, on `side` (-1 or 1, as the
/// ribbon's `across`), `along` metres down its centreline, where the
/// valley side beyond it rises by `rise` per metre and the ground just
/// beyond the water stands `room` metres above it. Outside a bend, or
/// wherever the valley side is steep (a rise of 0.3 to 0.7), the bank is
/// cut: a face of wet earth 0.2 to 1.5 m high, higher in deeper water and
/// tighter bends, under a turf lip hanging 0.1 to 0.4 m out over it, at
/// the back of a narrow margin. Inside a bend it shelves gently up from a
/// wide margin, at under 20 degrees. A bank never stands above the ground
/// beside it, and ground too low for a face has none. Face, lip and margin
/// wander with noise along the stream, about four widths long.
pub fn bank_shape(
  p: &ChannelPoint,
  side: f32,
  along: f32,
  (rise, room): (f32, f32),
  seed: u64,
) -> BankShape {
  let bend = p.curvature.abs();
  let outer = f32::from(u8::from(side * p.curvature < 0.0));
  let cut = (outer * smoothstep((bend - 0.05) / 0.2)).max(smoothstep((rise - 0.3) / 0.4))
    * smoothstep((room - 0.15) / 0.15);
  let wave = along / (4.0 * p.width).max(2.0);
  let lane = i32::from(side > 0.0);
  // `value_noise` along one lane, whose other two corners weigh nothing:
  // the same values for half the hashing.
  let cell = wave.floor() as i32;
  let t = smoothstep(wave - cell as f32);
  let noise = |salt: u64| {
    let a = crate::maths::hash_noise(seed ^ salt, cell, lane);
    let b = crate::maths::hash_noise(seed ^ salt, cell.wrapping_add(1), lane);
    0.5 + 0.5 * (a + (b - a) * t)
  };
  let cut_face = ((0.25 + 0.6 * p.depth) * (0.7 + 0.6 * noise(0x51)) * (1.0 + bend))
    .min(room + 0.1)
    .clamp(0.2, 1.5);
  let shelf = (0.1 + 0.3 * p.depth).min(0.4).min(room.max(0.05));
  let face = shelf + (cut_face - shelf) * cut;
  let spread = noise(0x52);
  let wide = (0.4 * p.width).clamp(0.3, 2.5) * (0.6 + 0.8 * spread);
  let narrow = 0.1 + 0.1 * spread;
  BankShape {
    cut,
    margin: wide + (narrow - wide) * cut,
    face,
    run: face * (3.0 + (0.1 - 3.0) * cut),
    overhang: cut * (0.1 + 0.3 * noise(0x53)),
    lip: cut * (0.25 * face).clamp(0.08, 0.25),
  }
}

/// Smooth a run of bank shapes along the stream, each the mean of those
/// within 1.5 widths of it (`arcs` metres down the centreline, `widths`
/// wide): a bend's cut bank grows and fades over a few widths, not from
/// one point to the next.
fn smooth_shapes(shapes: &mut [BankShape], arcs: &[f32], widths: &[f32]) {
  let raw = shapes.to_vec();

  for (i, shape) in shapes.iter_mut().enumerate() {
    let reach = 1.5 * widths[i];
    let (mut sum, mut count) = (BankShape::default(), 0.0);
    // The run's distances only grow, so its neighbours are one stretch.
    let first = arcs.partition_point(|&along| along < arcs[i] - reach);

    for (k, other) in raw.iter().enumerate().skip(first) {
      if arcs[k] - arcs[i] > reach {
        break;
      }

      if (arcs[k] - arcs[i]).abs() <= reach {
        count += 1.0;
        sum = sum.mix(*other, 1.0 / count);
      }
    }

    *shape = sum;
  }
}

/// One bank row: the drawn point it is built at, its side (-1 or 1), its
/// outward direction, its profile's shape and the ground's height under
/// the back of its turf.
#[derive(Clone, Copy, Debug)]
struct BankRow {
  point: u32,
  side: f32,
  outward: [f32; 2],
  shape: BankShape,
  ground: f32,
}

/// The rows of one stream's bank strips: runs of rows, each on one side.
#[derive(Clone, Debug, Default)]
struct BankStrips {
  rows: Vec<BankRow>,
  /// Each run's rows in `rows`.
  runs: Vec<std::ops::Range<usize>>,
  /// Keep every `stride`-th row of each run (and its last), to fit the
  /// map's budget.
  stride: usize,
}

impl BankStrips {
  fn rows(run: &std::ops::Range<usize>, stride: usize) -> usize {
    if stride <= 1 {
      run.len()
    } else {
      (run.len() - 1).div_ceil(stride) + 1
    }
  }

  fn vertices(&self, stride: usize) -> usize {
    self
      .runs
      .iter()
      .map(|run| BANK_PROFILE * Self::rows(run, stride))
      .sum()
  }
}

/// The ground under the back of a bank row's turf, never below the water.
fn turf_ground(
  map: &HeightMap,
  p: &ChannelPoint,
  outward: [f32; 2],
  shape: &BankShape,
  metres: f32,
) -> f32 {
  let out = (0.5 * p.width + shape.margin + shape.run + TURF_METRES) / metres;
  (full_detail_height(map, p.x + outward[0] * out, p.y + outward[1] * out) + 0.02)
    .max(p.level + 0.05)
}

/// A point's bank row on `side` (-1 or 1), or `None` where the stream
/// has no bank mesh. `along` is its distance down the centreline.
fn bank_row(
  map: &HeightMap,
  points: &[ChannelPoint],
  i: usize,
  side: f32,
  (metres, along, seed): (f32, f32, u64),
) -> Option<BankRow> {
  let n = points.len();
  let p = &points[i];
  let (prev, next) = (&points[i.saturating_sub(1)], &points[(i + 1).min(n - 1)]);
  let tangent = [next.x - prev.x, next.y - prev.y];
  let length = length2(tangent[0], tangent[1]);

  if p.width >= metres || p.falling || length <= 1e-5 {
    return None;
  }

  let outward = [-tangent[1] / length * side, tangent[0] / length * side];
  let ground = |out: f32| {
    let out = 0.5 * p.width / metres + out;
    full_detail_height(map, p.x + outward[0] * out, p.y + outward[1] * out)
  };
  // How steeply the valley side rises over the sample beyond the bank,
  // and how high the ground just beyond the water stands above it.
  let beside = ground(1.0 / metres);
  let rise = (ground(1.0) - ground(0.0)) / metres;
  let shape = bank_shape(p, side, along, (rise, beside - p.level), seed);
  Some(BankRow {
    point: i as u32,
    side,
    outward,
    shape,
    ground: turf_ground(map, p, outward, &shape, metres),
  })
}

/// Plan the banks beside a stream narrower than a heightmap sample, whose
/// points lie `arcs` metres down its centreline. Rows follow the drawn
/// points, their shapes smoothed along the stream ([`smooth_shapes`]),
/// except where the stream runs straight: a row is left out where the
/// heading turns by less than [`STRIP_MERGE_DEGREES`], the merged segment
/// is at most a sample long, and the ground under the dropped row's turf
/// and its face are within 5 cm of the straight bank's, so the bank
/// still lies on the drawn ground.
fn plan_bank_strips(
  map: &HeightMap,
  points: &[ChannelPoint],
  arcs: &[f32],
  metres: f32,
  seed: u64,
) -> BankStrips {
  let straight = STRIP_MERGE_DEGREES.to_radians().cos();
  let mut strips = BankStrips {
    rows: Vec::with_capacity(points.len() * 2),
    runs: Vec::new(),
    stride: 1,
  };

  for side in [-1.0f32, 1.0] {
    let mut rows: Vec<Option<BankRow>> = (0..points.len())
      .map(|i| bank_row(map, points, i, side, (metres, arcs[i], seed)))
      .collect();
    let mut i = 0;

    // Smooth each run of rows, then fit the turf to the smoothed shapes.
    while i < rows.len() {
      let end = (i..rows.len())
        .find(|k| rows[*k].is_none())
        .unwrap_or(rows.len());

      if end > i {
        let run: Vec<usize> = (i..end).collect();
        let mut shapes: Vec<BankShape> = run
          .iter()
          .filter_map(|k| rows[*k])
          .map(|r| r.shape)
          .collect();
        let along: Vec<f32> = run.iter().map(|k| arcs[*k]).collect();
        let widths: Vec<f32> = run.iter().map(|k| points[*k].width).collect();
        smooth_shapes(&mut shapes, &along, &widths);

        for (k, shape) in run.iter().zip(shapes) {
          if let Some(row) = &mut rows[*k] {
            row.shape = shape;
            row.ground = turf_ground(map, &points[*k], row.outward, &shape, metres);
          }
        }
      }

      i = end + 1;
    }

    let mut start = strips.rows.len();

    for i in 0..=points.len() {
      let Some(row) = rows.get(i).copied().flatten() else {
        // A run needs two rows to make a strip.
        if strips.rows.len() - start > 1 {
          strips.runs.push(start..strips.rows.len());
        } else {
          strips.rows.truncate(start);
        }

        start = strips.rows.len();
        continue;
      };

      // Drop the previous row if the strip runs straight through it.
      if let [a, b] = strips.rows[start.max(strips.rows.len().saturating_sub(2))..] {
        let (pa, pb, pc) = (
          &points[a.point as usize],
          &points[b.point as usize],
          &points[i],
        );
        let ab = [pb.x - pa.x, pb.y - pa.y];
        let bc = [pc.x - pb.x, pc.y - pb.y];
        let (lab, lbc) = (length2(ab[0], ab[1]), length2(bc[0], bc[1]));
        let chord = length2(pc.x - pa.x, pc.y - pa.y);
        let turn = (ab[0] * bc[0] + ab[1] * bc[1]) / (lab * lbc).max(1e-12);
        let along = lab / (lab + lbc).max(1e-12);
        let expected = a.ground + (row.ground - a.ground) * along;
        let shaped = (a.shape.face + (row.shape.face - a.shape.face) * along - b.shape.face).abs();

        if turn >= straight && chord <= 1.0 && (expected - b.ground).abs() <= 0.05 && shaped <= 0.05
        {
          strips.rows.pop();
        }
      }

      strips.rows.push(row);
    }
  }

  strips
}

/// Blend a tributary's banks into its main stem's over the last 2 w
/// before it joins: `shape` is the main stem's bank on the side the
/// tributary enters, at the join, `arcs` the tributary's distances down
/// its centreline and `width` its width at the join. At the join the
/// tributary's banks take the main stem's profile, so no seam shows.
fn blend_join(strips: &mut BankStrips, arcs: &[f32], width: f32, shape: BankShape) {
  let Some(end) = arcs.last() else {
    return;
  };
  let reach = (2.0 * width).max(1e-3);

  for row in &mut strips.rows {
    let left = end - arcs[row.point as usize];

    if left < reach {
      row.shape = shape.mix(row.shape, smoothstep(left / reach));
    }
  }
}

/// Fit every stream's banks into [`BANK_STRIP_BUDGET`]. Each stream
/// keeps every n-th row, with n the power of two at or above `scale /
/// visibility`, for the smallest `scale` that fits. The least visible
/// streams (the narrowest, slowest, lowest in Strahler order and furthest
/// from the map centre) are thinned first and most.
fn fit_bank_strips(strips: &mut [BankStrips], drawn: &[Vec<ChannelPoint>], map: &HeightMap) {
  let centre = [
    (map.metadata.width as f32 - 1.0) * 0.5,
    (map.metadata.height as f32 - 1.0) * 0.5,
  ];
  let reach = length2(centre[0], centre[1]).max(1.0);
  let visibility: Vec<f32> = drawn
    .iter()
    .map(|points| {
      let count = points.len().max(1) as f32;
      let width = points.iter().map(|p| p.width).sum::<f32>() / count;
      let speed = points.iter().map(|p| p.speed).sum::<f32>() / count;
      let away = points
        .get(points.len() / 2)
        .map_or(1.0, |p| length2(p.x - centre[0], p.y - centre[1]) / reach);
      let order = points.iter().map(|p| p.order).max().unwrap_or(0).max(1);
      (width * (0.2 + speed) * (1.5 - away) * f32::from(order)).max(1e-6)
    })
    .collect();
  let mut fit = |scale: f32| -> usize {
    strips
      .iter_mut()
      .zip(&visibility)
      .map(|(strip, visibility)| {
        strip.stride =
          ((scale / visibility).ceil().clamp(1.0, 65_536.0) as usize).next_power_of_two();
        strip.vertices(strip.stride)
      })
      .sum()
  };

  if fit(0.0) <= BANK_STRIP_BUDGET {
    return;
  }

  let (mut low, mut high) = (0.0, 1.0);

  while fit(high) > BANK_STRIP_BUDGET && high < 1e12 {
    (low, high) = (high, high * 4.0);
  }

  for _ in 0..48 {
    let middle = 0.5 * (low + high);

    if fit(middle) > BANK_STRIP_BUDGET {
      low = middle;
    } else {
      high = middle;
    }
  }

  fit(high);
}

/// Banks on either side of a stream narrower than a heightmap sample, at
/// the rows `strips` plans, `arcs` metres down its centreline: a wet
/// margin at the water's level, a face up to the turf lip and the turf
/// back to the ground (see [`BankShape`]). The heightmap has only a
/// trench one sample wide there, so the banks give it a real edge: the
/// mesh covers the trench's side and meets the drawn ground at the back
/// of its turf.
fn add_bank_strips(
  network: &mut RiverNetwork,
  map: &HeightMap,
  points: &[ChannelPoint],
  (strips, arcs): (&BankStrips, &[f32]),
  metres: f32,
  half: [f32; 2],
) {
  let profile = BANK_PROFILE as u32;

  for run in &strips.runs {
    let last = run.len() - 1;
    let kept = strips.rows[run.clone()]
      .iter()
      .enumerate()
      .filter(|(k, _)| strips.stride <= 1 || k % strips.stride == 0 || *k == last);

    for (row, (_, bank_row)) in kept.enumerate() {
      let BankRow {
        point,
        side,
        outward,
        shape,
        ..
      } = *bank_row;
      let p = &points[point as usize];
      let first = network.bank_vertices.len() as u32;
      let mut vertex = |(across, offset, height): (f32, f32, f32)| {
        let x = p.x + outward[0] * offset / metres;
        let y = p.y + outward[1] * offset / metres;
        let world = to_world([x, y], metres, half);
        network.bank_vertices.push(BankVertex {
          position: [world[0], height, world[1]],
          outward,
          params: [across, arcs[point as usize], p.speed, side],
          shape: [shape.cut, p.width],
        });
      };
      let edge = 0.5 * p.width;

      for (k, [offset, rise]) in shape.points().into_iter().enumerate() {
        vertex((k as f32, edge + offset, p.level + rise));
      }

      vertex((
        4.0,
        edge + shape.margin + shape.run + TURF_METRES,
        turf_ground(map, p, outward, &shape, metres),
      ));

      if row > 0 {
        // Wound towards the water on one side and away on the other: the
        // pass draws both faces.
        for k in 0..profile - 1 {
          let (a, b) = (first - profile + k, first + k);
          network
            .bank_indices
            .extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
      }
    }
  }
}

/// Record the slow runs of a stream narrower than a heightmap sample, for
/// reeds along its true banks.
fn add_brooks(network: &mut RiverNetwork, points: &[ChannelPoint], metres: f32, half: [f32; 2]) {
  let mut run: Vec<[f32; 3]> = Vec::new();

  for p in points {
    if p.width < metres && p.speed < REED_SPEED && !p.falling {
      let world = to_world([p.x, p.y], metres, half);
      run.push([world[0], world[1], 0.5 * p.width]);
    } else if run.len() > 1 {
      network.brooks.push(std::mem::take(&mut run));
    } else {
      run.clear();
    }
  }

  if run.len() > 1 {
    network.brooks.push(run);
  }
}

/// Most points one simplified ribbon segment may span.
const MAX_RUN: usize = 64;

/// Drop the dense points of a ribbon that lie on a straight, uniform run:
/// where the centreline strays less than 0.05 w, or a twentieth of a
/// sample for streams narrower than one (the carve itself follows the
/// centreline to a tenth of one), from the chord between the points kept
/// either side, width and depth differ from their interpolation along the
/// chord by less than 2 %, speed and slope by less than 10 %, level by
/// less than a tenth of the depth, and curvature by 0.05. The ends, and of
/// the channel's own points (`original`; the rest
/// subdivide steep segments) those at falls, sharp bends (|curvature|
/// over 0.2) and those `anchored` names (joins and lake shores), are
/// always kept.
fn simplify_ribbon(
  points: &[ChannelPoint],
  original: &[bool],
  metres: f32,
  anchored: &dyn Fn(&ChannelPoint) -> bool,
) -> Vec<ChannelPoint> {
  let n = points.len();

  if n <= 2 {
    return points.to_vec();
  }

  let keep: Vec<bool> = (0..n)
    .map(|i| {
      let p = &points[i];
      i == 0
        || i == n - 1
        || (original[i]
          && (p.falling
            || points[i - 1].falling
            || points[i + 1].falling
            || p.curvature.abs() > 0.2
            || anchored(p)))
    })
    .collect();
  let close = |value: f32, expected: f32, scale: f32| (value - expected).abs() <= 0.02 * scale;
  // Speed and slope only shade the water, and vary smoothly along a curve.
  let loose = |value: f32, expected: f32, scale: f32| (value - expected).abs() <= 0.1 * scale;
  // Whether every point between `a` and `c` is where the chord from `a` to
  // `c` would put it.
  let fits = |a: usize, c: usize| {
    let (pa, pc) = (&points[a], &points[c]);
    let chord = [(pc.x - pa.x) * metres, (pc.y - pa.y) * metres];
    let length = length2(chord[0], chord[1]).max(1e-4);

    (a + 1..c).all(|k| {
      let p = &points[k];
      let offset = [(p.x - pa.x) * metres, (p.y - pa.y) * metres];
      let along =
        ((offset[0] * chord[0] + offset[1] * chord[1]) / (length * length)).clamp(0.0, 1.0);
      let off_chord = (offset[0] * chord[1] - offset[1] * chord[0]).abs() / length;
      let lerp = |u: f32, v: f32| u + (v - u) * along;

      off_chord < 0.05 * p.width.max(metres)
        && close(p.width, lerp(pa.width, pc.width), p.width)
        && close(p.depth, lerp(pa.depth, pc.depth), p.depth)
        && loose(p.speed, lerp(pa.speed, pc.speed), p.speed)
        && loose(p.slope, lerp(pa.slope, pc.slope), p.slope.max(0.005))
        && loose(p.level, lerp(pa.level, pc.level), p.depth)
        && (p.curvature - lerp(pa.curvature, pc.curvature)).abs() <= 0.05
        && (p.celsius - lerp(pa.celsius, pc.celsius)).abs() <= 0.1
    })
  };
  // The first point at or after each index that must be kept (the last
  // point always is), so a run's limit is found without rescanning.
  let mut next_keep = vec![n - 1; n];

  for i in (0..n - 1).rev() {
    next_keep[i] = if keep[i] { i } else { next_keep[i + 1] };
  }

  let mut kept = Vec::with_capacity(n);
  kept.push(points[0]);
  let mut a = 0;

  while a < n - 1 {
    // The furthest a run may reach: the next kept point, the end, or
    // MAX_RUN points on.
    let limit = next_keep[a + 1].min(a + MAX_RUN);

    // The longest run that fits, by bisection: checking every length
    // would cost the square of the run.
    let (mut good, mut bad) = (a + 1, limit + 1);

    if fits(a, limit) {
      good = limit;
    } else {
      while bad - good > 1 {
        let mid = (good + bad) / 2;

        if fits(a, mid) {
          good = mid;
        } else {
          bad = mid;
        }
      }
    }

    kept.push(points[good]);
    a = good;
  }

  kept
}

/// Still water in a cut-off meander loop.
fn add_oxbow(network: &mut RiverNetwork, oxbow: &Oxbow, metres: f32, half: [f32; 2]) {
  let n = oxbow.points.len();
  let world: Vec<[f32; 2]> = oxbow
    .points
    .iter()
    .map(|p| to_world(*p, metres, half))
    .collect();
  let half_width = 0.65 * oxbow.width;
  let rows: Vec<_> = (0..n)
    .map(|i| {
      let prev = world[i.saturating_sub(1)];
      let next = world[(i + 1).min(n - 1)];
      let tangent = [next[0] - prev[0], next[1] - prev[1]];
      let length = length2(tangent[0], tangent[1]).max(1e-4);
      StripRow {
        centre: world[i],
        side: [-tangent[1] / length, tangent[0] / length],
        half: [half_width; 2],
        level: oxbow.surface,
        flow: [0.0, 0.0],
        extra: [0.0, 0.0, 0.0, oxbow.celsius],
        along: 0.0,
        swirl: 0.0,
        step: 0.0,
      }
    })
    .collect();
  push_strip(
    &mut network.vertices,
    &mut network.indices,
    WATER_KIND_LAKE,
    &mut rows.into_iter(),
    &|_| false,
  );
}

/// Flat lake surfaces: one quad per flow cell, grown by a ring of cells so
/// the surface always reaches past the shore, where the shader fades it
/// out by depth.
fn add_lakes(network: &mut RiverNetwork, hydrology: &Hydrology, map: &HeightMap, half: [f32; 2]) {
  let metres = map.metadata.metres_per_sample.max(0.001);
  let map_width = map.metadata.width;
  let stride = hydrology.stride;
  let half_cell = stride as f32 * metres * 0.5;
  let shore_spacing = (32.0 / hydrology.cell_metres).max(1.0) as usize;

  let mut marked = vec![false; hydrology.ground.len()];

  for lake in &hydrology.lakes {
    let id = hydrology.lake[lake.cells[0] as usize];
    let mut covered = Vec::new();
    let mut shore = Vec::new();

    for cell in &lake.cells {
      let mut on_shore = false;

      for n in std::iter::once(*cell).chain(crate::terrain::drainage::neighbours(
        hydrology.width,
        hydrology.height,
        *cell,
      )) {
        on_shore |= hydrology.lake[n as usize] != id;

        if !marked[n as usize] {
          marked[n as usize] = true;
          covered.push(n);
        }
      }

      if on_shore {
        shore.push(*cell);
      }
    }

    // Lakes never touch (they would be one lake), but their rings of
    // shore cells may.
    for cell in &covered {
      marked[*cell as usize] = false;
    }

    let extra = [0.0, 0.0, lake.depth, lake.celsius];

    for cell in covered {
      let (sx, sy) = hydrology.sample_xy(cell);
      let centre = to_world([sx as f32, sy as f32], metres, half);
      let first = network.vertices.len() as u32;

      for (dx, dz) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        network.vertices.push(WaterVertex {
          position: [
            centre[0] + dx * half_cell,
            lake.surface,
            centre[1] + dz * half_cell,
          ],
          flow: [0.0, 0.0],
          params: [WATER_KIND_LAKE, 0.0, 0.0, 0.0],
          extra,
          swirl: 0.0,
        });
      }

      network.indices.extend_from_slice(&[
        first,
        first + 2,
        first + 1,
        first + 1,
        first + 2,
        first + 3,
      ]);

      // Samples in this cell below the surface are under water.
      let radius = (stride / 2) as i32;

      for oy in -radius..=radius {
        for ox in -radius..=radius {
          let x = sx as i32 + ox;
          let y = sy as i32 + oy;

          if x >= 0 && y >= 0 && (x as u32) < map_width && (y as u32) < map.metadata.height {
            let index = (y as u32 * map_width + x as u32) as usize;

            if map.heights[index] < lake.surface {
              network.mask[index] = true;
            }
          }
        }
      }
    }

    network.lakes.push(LakeSummary {
      surface: lake.surface,
      celsius: lake.celsius,
      endorheic: lake.endorheic,
      shore: shore
        .iter()
        .step_by(shore_spacing)
        .map(|cell| {
          let (x, y) = hydrology.sample_xy(*cell);
          [x as f32, y as f32]
        })
        .collect(),
    });
  }
}

/// Rows down a waterfall sheet, and down each step of a cascade.
const FALL_ROWS: usize = 12;
const CASCADE_STEP_ROWS: usize = 6;

/// A churned plunge pool at `foot`: a full pool when `depth` is over 0,
/// or, below the upper steps of a cascade, a churned film on the ground.
/// Its water stands only as high as where it spills: the lowest ground
/// just outside its rim, or the water in its outlet channel, and at most
/// the foot. Below that it is flat; above it, and all over where the bowl
/// holds nothing, as on a slope, it is a thin film over the ground, so it
/// never stands out as a shelf. The shader fades it towards its rim, so
/// where the pool is smaller than a heightmap sample it does not end in a
/// hard edge.
#[allow(clippy::too_many_arguments)]
fn add_pool(
  network: &mut RiverNetwork,
  map: &HeightMap,
  fall: &Fall,
  foot: [f32; 2],
  foot_level: f32,
  radius: f32,
  depth: f32,
  energy: f32,
  metres: f32,
  half: [f32; 2],
) {
  let ring = radius / metres + 0.5;
  let mut level = if depth > 0.0 {
    foot_level
  } else {
    f32::NEG_INFINITY
  };

  if depth > 0.0 {
    for k in 0..POOL_SEGMENTS {
      let angle = k as f32 / POOL_SEGMENTS as f32 * std::f32::consts::TAU;
      let (cos, sin) = (angle.cos(), angle.sin());
      let ground = height_at(map, foot[0] + cos * ring, foot[1] + sin * ring);
      let outlet = cos * fall.direction[0] + sin * fall.direction[1] > 0.77;
      level = level.min(if outlet {
        ground + channel_depth(fall.discharge)
      } else {
        ground
      });
    }
  }

  let surface = |x: f32, y: f32| {
    let ground = height_at(map, x, y).max(full_detail_height(map, x, y));
    foot_level.min(level.max(ground + POOL_FILM_METRES))
  };
  let held = (level - (foot_level - depth)).max(0.0);
  let centre = network.vertices.len() as u32;
  let world = to_world(foot, metres, half);
  // Bowl depth, the fall's energy (drop times discharge) and, where
  // frozen water expects it, the temperature.
  let pool = [held, energy, 0.0, fall.celsius];
  network.vertices.push(WaterVertex {
    position: [world[0], surface(foot[0], foot[1]) + 0.02, world[1]],
    flow: [0.0, 0.0],
    params: [WATER_KIND_POOL, 0.0, 0.0, 0.0],
    extra: pool,
    swirl: 0.0,
  });

  let rings = ((3.0 * radius / metres).ceil() as usize).clamp(2, POOL_RINGS);

  for ring in 1..=rings {
    let across = ring as f32 / rings as f32;
    let r = radius * across;

    for k in 0..POOL_SEGMENTS {
      let angle = k as f32 / POOL_SEGMENTS as f32 * std::f32::consts::TAU;
      let (cos, sin) = (angle.cos(), angle.sin());
      let level = surface(foot[0] + cos * r / metres, foot[1] + sin * r / metres);
      network.vertices.push(WaterVertex {
        position: [world[0] + cos * r, level + 0.02, world[1] + sin * r],
        flow: [cos, sin],
        params: [WATER_KIND_POOL, across, 0.0, 0.0],
        extra: pool,
        swirl: 0.0,
      });
    }
  }

  let segments = POOL_SEGMENTS as u32;

  for k in 0..segments {
    let next = (k + 1) % segments;
    network
      .indices
      .extend_from_slice(&[centre, centre + 1 + next, centre + 1 + k]);

    for ring in 1..rings as u32 {
      let inner = centre + 1 + (ring - 1) * segments;
      let outer = inner + segments;
      network.indices.extend_from_slice(&[
        inner + k,
        inner + next,
        outer + k,
        inner + next,
        outer + next,
        outer + k,
      ]);
    }
  }
}

/// A waterfall or cascade: churned plunge pools (with the other water
/// surfaces), and in a buffer of their own, one falling sheet over every
/// step and one mist cloud at the bottom. Trickles have none of these:
/// their step is whitewater on the river ribbon.
fn add_fall(
  network: &mut RiverNetwork,
  map: &HeightMap,
  fall: &Fall,
  metres: f32,
  half: [f32; 2],
  seed: u64,
) {
  if fall.trickle {
    return;
  }

  let height = fall.height();
  let single = [FallStep {
    lip: fall.lip,
    lip_level: fall.lip_level,
    foot: fall.foot,
    foot_level: fall.foot_level,
    pool_radius: fall.pool_radius,
  }];
  let steps: &[FallStep] = if fall.steps.is_empty() {
    &single
  } else {
    &fall.steps
  };

  for step in &steps[..steps.len() - 1] {
    let drop = step.lip_level - step.foot_level;
    let energy = drop * fall.discharge;
    add_pool(
      network,
      map,
      fall,
      step.foot,
      step.foot_level,
      step.pool_radius,
      0.0,
      energy,
      metres,
      half,
    );
  }

  add_pool(
    network,
    map,
    fall,
    fall.foot,
    fall.foot_level,
    fall.pool_radius,
    fall.pool_depth,
    height * fall.discharge,
    metres,
    half,
  );

  let speed = fall.speed.max(0.5);
  let columns = ((fall.width / 4.0).ceil() as usize).max(3);
  let per_step = if steps.len() > 1 {
    CASCADE_STEP_ROWS
  } else {
    FALL_ROWS
  };
  let highest = steps
    .iter()
    .map(|step| step.lip_level - step.foot_level)
    .fold(0.0f32, f32::max);
  let impact = (2.0 * GRAVITY * highest).sqrt();
  let foot = to_world(fall.foot, metres, half);
  let vertices = &mut network.fall_vertices;
  let indices = &mut network.fall_indices;

  // The sheet follows the path of water leaving each lip, x = v t and
  // y = -g t^2 / 2, but never cuts into the rock: where the face is less
  // than vertical it is pushed out to lie just over it. Between the steps
  // of a cascade it runs straight from one foot to the next lip.
  let mut rows: Vec<([f32; 2], f32, [f32; 2])> = Vec::new();

  for step in steps {
    let drop = step.lip_level - step.foot_level;
    let fall_time = (2.0 * drop / GRAVITY).sqrt();
    let dx = step.foot[0] - step.lip[0];
    let dy = step.foot[1] - step.lip[1];
    let run = length2(dx, dy).max(1e-4);
    let direction = [dx / run, dy / run];
    let reach = (speed * fall_time).max(run * metres);

    for k in 0..=per_step {
      let along = reach * k as f32 / per_step as f32;
      let t = along / speed;
      let projectile = if t <= fall_time {
        step.lip_level - 0.5 * GRAVITY * t * t
      } else {
        step.foot_level
      };
      let sx = step.lip[0] + direction[0] * along / metres;
      let sy = step.lip[1] + direction[1] * along / metres;
      let rock = height_at(map, sx, sy);
      let y = if k == per_step {
        step.foot_level
      } else {
        projectile.max(rock + 0.3).max(step.foot_level)
      };
      rows.push(([sx, sy], y, direction));
    }
  }

  let distance = |a: &([f32; 2], f32, [f32; 2]), b: &([f32; 2], f32, [f32; 2])| {
    length2(
      length2(b.0[0] - a.0[0], b.0[1] - a.0[1]) * metres,
      b.1 - a.1,
    )
  };
  let total = rows
    .windows(2)
    .map(|pair| distance(&pair[0], &pair[1]))
    .sum::<f32>()
    .max(0.01);
  let mut travelled = 0.0;
  let first = vertices.len() as u32;

  for (k, row) in rows.iter().enumerate() {
    if k > 0 {
      travelled += distance(&rows[k - 1], row);
    }

    let ([sx, sy], y, direction) = *row;
    let side = [-direction[1], direction[0]];

    for column in 0..=columns {
      let across = column as f32 / columns as f32 * 2.0 - 1.0;
      let offset = across * fall.width * 0.5 / metres;
      let world = to_world([sx + side[0] * offset, sy + side[1] * offset], metres, half);
      vertices.push(WaterVertex {
        position: [world[0], y, world[1]],
        flow: [direction[0] * speed, direction[1] * speed],
        params: [WATER_KIND_FALL, across, impact, 0.0],
        extra: [travelled, total, fall.celsius, height],
        swirl: 0.0,
      });
    }
  }

  let stride = columns as u32 + 1;

  for k in 0..rows.len().saturating_sub(1) as u32 {
    for column in 0..columns as u32 {
      let a = first + k * stride + column;
      let b = a + 1;
      let c = a + stride;
      let d = c + 1;
      indices.extend_from_slice(&[a, c, b, b, c, d]);
    }
  }

  // Mist: 16 to 64 camera-facing sprites, more for bigger falls.
  let sprites = (16.0 + (fall.discharge * height).sqrt() * 4.0).clamp(16.0, 64.0) as u32;
  // Bigger falls throw up bigger clouds; a trickle only a light mist.
  let size = (0.25 * height + fall.width * 0.5)
    .min(1.0 + 20.0 * fall.discharge.sqrt())
    .clamp(1.5, 25.0);

  for k in 0..sprites {
    let hash = crate::maths::hash_u64(seed ^ ((k as u64) << 32) ^ (fall.foot[0].to_bits() as u64));
    let unit = |shift: u32| ((hash >> shift) & 0xffff) as f32 / 65_535.0;
    let angle = unit(0) * std::f32::consts::TAU;
    let distance = unit(16).sqrt() * fall.pool_radius * 0.8;
    let centre = [
      foot[0] + angle.cos() * distance,
      foot[1] + angle.sin() * distance,
    ];
    let first = vertices.len() as u32;

    for (cx, cy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
      vertices.push(WaterVertex {
        position: [centre[0], fall.foot_level, centre[1]],
        flow: [cx, cy],
        params: [
          WATER_KIND_SPRAY,
          unit(32),
          size * (0.6 + 0.4 * unit(48)),
          0.0,
        ],
        extra: [height, fall.discharge, fall.celsius, fall.pool_radius],
        swirl: 0.0,
      });
    }

    indices.extend_from_slice(&[first, first + 2, first + 1, first + 1, first + 2, first + 3]);
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::terrain::heightmap::update_stats;
  use vista_types::TerrainMetadata;

  /// A map 31 m across, millimetres to metres a sample, cannot hold even
  /// the narrowest channel, 0.6 m wide and deep: water drawn on it hung
  /// a hundred metres over the ground. It gets no rivers instead.
  #[test]
  fn a_map_too_small_for_a_channel_gets_no_rivers() {
    for metres in [0.001, 0.5] {
      let size = 64;
      let heights: Vec<f32> = (0..size * size)
        .map(|index| 10.0 + (index % size) as f32 * 0.3 + (index / size) as f32 * 0.2)
        .collect();
      let metadata = TerrainMetadata {
        metres_per_sample: metres,
        vertical_scale: 1.0,
        ..TerrainMetadata::default()
      };
      let mut map = HeightMap::from_values(
        size,
        size,
        heights.clone(),
        vec![false; heights.len()],
        metadata,
      )
      .unwrap();
      // An inflow, so there is a river to place.
      let options = RiverOptions {
        inflow: vista_types::RiverInflows::List(vec![vista_types::RiverInflow {
          position: [0.0, 0.0],
          discharge_cubic_metres_per_second: 10.0,
        }]),
        ..RiverOptions::default()
      };
      let network = build_river_network(
        &mut map,
        &options,
        RiverSources {
          surface: &[],
          seed: 1,
          painted: Vec::new(),
          record: crate::terrain::channels::CarveRecord::new(heights.len()),
        },
      );
      assert!(network.vertices.is_empty() && network.reaches.is_empty());
      assert_eq!(map.heights, heights);
    }
  }
  #[test]
  fn lakes_freeze_below_zero_margins_first_and_fully_at_minus_two() {
    assert_eq!(freeze_fraction(-3.0, LAKE_FREEZE_CELSIUS), 1.0);
    assert_eq!(freeze_fraction(1.0, LAKE_FREEZE_CELSIUS), 0.0);
    let partial = freeze_fraction(-1.0, LAKE_FREEZE_CELSIUS);
    assert!(partial > 0.0 && partial < 1.0);
    assert_eq!(freeze_fraction(-6.0, RIVER_FREEZE_CELSIUS), 0.5);
    assert_eq!(freeze_fraction(-10.0, FALL_FREEZE_CELSIUS), 1.0);
  }

  /// CPU twins of the river functions in `water.wgsl`.
  fn velocity_profile(across: f32, curvature: f32) -> f32 {
    let c = 0.35 * curvature.abs();
    let off = across + c * curvature.signum() * f32::from(u8::from(curvature != 0.0));
    (1.15 - 0.45 * off * off) / (1.0 - 0.45 * c * c)
  }

  fn variance_preserving(a: f32, b: f32, w: f32) -> f32 {
    (a * (1.0 - w) + b * w) / (w * w + (1.0 - w) * (1.0 - w)).sqrt()
  }

  fn curl_noise(psi: &dyn Fn(f32, f32) -> f32, q: [f32; 2]) -> [f32; 2] {
    let e = 0.25;
    [
      (psi(q[0], q[1] + e) - psi(q[0], q[1] - e)) / (2.0 * e),
      (psi(q[0] - e, q[1]) - psi(q[0] + e, q[1])) / (2.0 * e),
    ]
  }

  #[test]
  fn the_velocity_profile_keeps_its_mean_and_hugs_the_outer_bank() {
    for curvature in [0.0f32, 0.5, 1.0, -1.0] {
      let n = 2001;
      let across = |k: usize| -1.0 + 2.0 * k as f32 / (n - 1) as f32;
      let mean = (0..n)
        .map(|k| velocity_profile(across(k), curvature))
        .sum::<f32>()
        / n as f32;
      assert!((mean - 1.0).abs() < 0.01, "mean {mean} at {curvature}");

      if curvature != 0.0 {
        let peak = (0..n)
          .max_by(|a, b| {
            velocity_profile(across(*a), curvature)
              .total_cmp(&velocity_profile(across(*b), curvature))
          })
          .map(across)
          .unwrap();
        // The outer bank is on the side `-sign(curvature)`, as in the shader.
        assert!(
          peak * -curvature.signum() > 0.0,
          "peak at {peak} for {curvature}"
        );
      }
    }
  }

  #[test]
  fn the_flow_map_fade_keeps_ripple_contrast() {
    let field = |seed: u64| -> Vec<f32> {
      (0..4096u64)
        .map(|i| crate::maths::hash_u64(seed ^ i) as f32 / u64::MAX as f32 * 2.0 - 1.0)
        .collect()
    };
    let (a, b) = (field(1), field(2));
    let deviation = |values: &[f32]| {
      let mean = values.iter().sum::<f32>() / values.len() as f32;
      (values.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / values.len() as f32).sqrt()
    };
    let reference = deviation(&a);

    for step in 0..=20 {
      let w = step as f32 / 20.0;
      let blended: Vec<f32> = a
        .iter()
        .zip(&b)
        .map(|(a, b)| variance_preserving(*a, *b, w))
        .collect();
      let ratio = deviation(&blended) / reference;
      assert!((ratio - 1.0).abs() <= 0.05, "{ratio} at {w}");
    }
  }

  #[test]
  fn flow_map_phases_never_fade_all_at_once() {
    // A 64 x 64 grid of pixels 6 m apart, each offset by a smooth noise
    // times 4, as in the shader.
    let offsets: Vec<f32> = (0..64 * 64)
      .map(|i| {
        let (x, y) = ((i % 64) as f32 * 6.0, (i / 64) as f32 * 6.0);
        (crate::maths::value_noise(7, x / 12.0, y / 12.0) * 0.5 + 0.5) * 4.0
      })
      .collect();

    for step in 0..200 {
      let t = step as f32 * 0.05;
      let late = offsets
        .iter()
        .filter(|offset| (t * 0.35 + **offset).fract() >= 0.9)
        .count() as f32
        / offsets.len() as f32;
      assert!((0.08..=0.12).contains(&late), "{late} at {t} s");
    }
  }

  #[test]
  fn curl_noise_is_divergence_free() {
    let psi =
      |x: f32, y: f32| (1.3 * x + 0.4).sin() * (0.7 * y).cos() + 0.5 * (2.1 * y - 0.9 * x).sin();

    for k in 0..100 {
      let q = [k as f32 * 0.37, k as f32 * 0.21 + 1.0];
      let h = 0.25;
      let at = |dx: f32, dy: f32| curl_noise(&psi, [q[0] + dx, q[1] + dy]);
      let divergence =
        (at(h, 0.0)[0] - at(-h, 0.0)[0]) / (2.0 * h) + (at(0.0, h)[1] - at(0.0, -h)[1]) / (2.0 * h);
      let magnitude = length2(at(0.0, 0.0)[0], at(0.0, 0.0)[1]).max(1e-3);
      assert!(
        divergence.abs() < 1e-3 * magnitude.max(1.0),
        "{divergence} at {q:?}"
      );
    }
  }

  #[test]
  fn the_inner_edge_of_a_tight_bend_never_folds() {
    // A half circle of radius 0.6 w, w = 10 m, on a 1 m grid.
    let points: Vec<ChannelPoint> = (0..41)
      .map(|i| {
        let angle = i as f32 / 40.0 * std::f32::consts::PI;
        ChannelPoint {
          x: 50.0 + 6.0 * angle.cos(),
          y: 50.0 + 6.0 * angle.sin(),
          width: 10.0,
          ..straight_reach(1)[0]
        }
      })
      .collect();
    let mut network = RiverNetwork::default();
    let still = vec![0.0; points.len()];
    push_ribbon(
      &mut network,
      (&points, &still, &still),
      1.0,
      [0.0, 0.0],
      1.0,
    );
    let rows: Vec<[[f32; 3]; 2]> = network
      .vertices
      .chunks(2)
      .map(|pair| [pair[0].position, pair[1].position])
      .collect();

    // The turn is to the left, so the inner edge is on the `across = 1`
    // side; each of its vertices lies ahead of the last along the bend.
    for i in 1..rows.len() - 1 {
      let p = &points[i];
      let tangent = [
        points[i + 1].x - points[i - 1].x,
        points[i + 1].y - points[i - 1].y,
      ];
      let (a, b) = (rows[i][1], rows[i + 1][1]);
      let ahead = (b[0] - a[0]) * tangent[0] + (b[2] - a[2]) * tangent[1];
      assert!(ahead >= -1e-4, "row {i} folds back at {}, {}", p.x, p.y);
    }
  }

  #[test]
  fn eddies_sit_below_bend_apexes_and_falls() {
    // Straight, then a left bend peaking at point 20, then straight, then
    // a fall over points 50 to 54; points half a sample apart on a 10 m
    // grid, 3 m wide.
    let mut points = straight_reach(80);

    for (i, p) in points.iter_mut().enumerate() {
      p.curvature = 0.8 * (1.0 - (i as f32 - 20.0).abs() / 6.0).max(0.0);
      p.falling = (50..=54).contains(&i);
    }

    let s: Vec<f32> = (0..80).map(|i| i as f32 * 5.0).collect();
    let swirl = swirls(&points, &s);

    for (i, value) in swirl.iter().enumerate() {
      // Eddies rise from nothing at the apex and the foot (5 m a point,
      // over 9 m and 6 m).
      let below_apex = i == 21;
      let below_fall = i == 56;
      assert_eq!(
        *value != 0.0,
        below_apex || below_fall,
        "point {i}: {value}"
      );
    }

    // On the inner bank of the left turn: the `across = 1` side.
    assert!(swirl[21] > 0.0);
  }

  /// A straight reach along x with a steady fall, `n` points half a
  /// sample apart.
  fn straight_reach(n: usize) -> Vec<ChannelPoint> {
    (0..n)
      .map(|i| ChannelPoint {
        x: 4.0 + i as f32 * 0.5,
        y: 20.0,
        level: 50.0 - i as f32 * 0.01,
        bed: 49.0 - i as f32 * 0.01,
        width: 3.0,
        depth: 1.0,
        discharge: 1.0,
        slope: 0.002,
        speed: 0.8,
        curvature: 0.0,
        celsius: 12.0,
        rapids: 0.0,
        falling: false,
        order: 1,
      })
      .collect()
  }

  #[test]
  fn a_ribbon_has_no_rows_inside_a_fall() {
    let mut points = straight_reach(12);

    for p in &mut points[3..8] {
      p.falling = true;
    }

    let mut network = RiverNetwork::default();
    let still = vec![0.0; points.len()];
    push_ribbon(
      &mut network,
      (&points, &still, &still),
      1.0,
      [0.0, 0.0],
      1.0,
    );

    // Rows 4 to 6 have a falling quad on both sides; rows 3 and 7, at the
    // lip and the foot, still carry the quads either side of the fall.
    assert_eq!(network.vertices.len(), 2 * (12 - 3));
    let mut used = vec![false; network.vertices.len()];

    for &index in &network.indices {
      used[index as usize] = true;
    }

    assert!(used.iter().all(|&used| used), "{used:?}");
    // Quads 0 to 2 above the fall and 7 to 10 below it.
    assert_eq!(network.indices.len(), 6 * 7);
  }

  /// A 64 x 48 flat map on a 10 m grid at 50.2 m, with every sample
  /// marked as touched by the channel stage, and a straight river 20 m
  /// wide along row 20 at `speed`, its surface at 50 m.
  fn bed_scene(speed: f32, curvature: f32) -> (HeightMap, Vec<Reach>, Vec<bool>) {
    let map = HeightMap::flat(
      64,
      48,
      50.2,
      TerrainMetadata {
        width: 64,
        height: 48,
        metres_per_sample: 10.0,
        sea_level_metres: 0.0,
        ..TerrainMetadata::default()
      },
    );
    let points = straight_reach(100)
      .into_iter()
      .map(|mut point| {
        point.width = 20.0;
        point.level = 50.0;
        point.speed = speed;
        point.curvature = curvature;
        point
      })
      .collect();
    let mask = vec![true; 64 * 48];
    (
      map,
      vec![Reach {
        points,
        ..Reach::default()
      }],
      mask,
    )
  }

  fn bed_at(bed: &[(u32, [u8; 4])], x: u32, y: u32) -> [u8; 4] {
    bed
      .iter()
      .find(|(index, _)| *index == y * 64 + x)
      .map_or([0; 4], |(_, weights)| *weights)
  }

  #[test]
  fn river_beds_sort_gravel_sand_and_mud_by_speed() {
    for (speed, slot) in [(1.5, 0), (0.7, 1), (0.2, 2)] {
      let (map, reaches, mask) = bed_scene(speed, 0.0);
      let bed = bed_materials(&map, &reaches, &[], &mask);
      let beside = bed_at(&bed, 30, 21);
      assert!(beside[slot] > 200, "{speed} m/s: {beside:?}");
      // Far from the water the ground is left alone.
      assert_eq!(bed_at(&bed, 30, 40), [0; 4]);
    }
  }

  #[test]
  fn river_beds_only_touch_changed_samples_by_wide_rivers() {
    let (map, reaches, _) = bed_scene(1.5, 0.0);
    let untouched = vec![false; 64 * 48];
    assert!(bed_materials(&map, &reaches, &[], &untouched).is_empty());
    let carved = [(21 * 64 + 30, 51.0)];
    let bed = bed_materials(&map, &reaches, &carved, &untouched);
    assert_eq!(bed.len(), 1);

    let (map, mut reaches, mask) = bed_scene(1.5, 0.0);
    for point in &mut reaches[0].points {
      point.width = 5.0;
    }
    assert!(bed_materials(&map, &reaches, &[], &mask).is_empty());
  }

  #[test]
  fn point_bars_reach_out_on_the_inner_side_of_bends() {
    // Turning left (positive curvature), the inner side is +y.
    let (map, reaches, mask) = bed_scene(0.7, 0.5);
    let bed = bed_materials(&map, &reaches, &[], &mask);
    let sum = |w: [u8; 4]| w.iter().map(|v| u32::from(*v)).sum::<u32>();
    assert!(sum(bed_at(&bed, 30, 20 + 3)) > 200);
    assert_eq!(sum(bed_at(&bed, 30, 20 - 3)), 0);
  }

  #[test]
  fn powerful_steep_reaches_get_rock_banks_at_any_width() {
    let (map, mut reaches, mask) = bed_scene(2.0, 0.0);
    for point in &mut reaches[0].points {
      point.width = 5.0;
      point.discharge = 10.0;
      point.slope = 0.05;
    }
    let bed = bed_materials(&map, &reaches, &[], &mask);
    let beside = bed_at(&bed, 30, 21);
    assert!(beside[3] > 100 && beside[..3] == [0; 3], "{beside:?}");
  }

  /// CPU port of `reflection_fade` in `water.wgsl`.
  fn reflection_fade(uv: [f32; 2], travelled: f32, reach: f32) -> f32 {
    let edge = uv[0].min(1.0 - uv[0]).min(uv[1].min(1.0 - uv[1]));
    smoothstep(edge / 0.1) * (1.0 - smoothstep((travelled / reach - 0.6) / 0.4))
  }

  #[test]
  fn screen_reflections_fade_at_the_edges_and_the_end_of_the_ray() {
    // The ray steps of `trace_reflection`: 16, growing, out to its reach.
    let reach: f32 = 4000.0;
    let steps: Vec<f32> = (1..=16)
      .map(|i| 2.0 * (reach / 2.0).powf(i as f32 / 16.0))
      .collect();
    assert!((steps[15] - reach).abs() < 0.1);
    assert!(steps[0] < 4.0);
    assert!(steps.windows(3).all(|w| w[2] - w[1] > w[1] - w[0]));

    assert_eq!(reflection_fade([0.5, 0.5], 100.0, reach), 1.0);
    for edge in [[0.0, 0.5], [1.0, 0.5], [0.5, 0.0], [0.5, 1.0]] {
      assert_eq!(reflection_fade(edge, 100.0, reach), 0.0);
    }
    assert_eq!(reflection_fade([0.5, 0.5], reach, reach), 0.0);
    assert!(reflection_fade([0.05, 0.5], 100.0, reach) > 0.0);
  }

  #[test]
  fn wet_banks_measure_from_the_true_edge_of_a_narrow_stream() {
    let size = 64u32;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 30.0,
      ..TerrainMetadata::default()
    };
    let map = HeightMap::flat(size, size, 5.0, metadata);
    // A 2 m brook along row 20, which marks its samples as water, and a
    // lake filling the samples of rows 40 to 44.
    let brook: Vec<ChannelPoint> = flat_brook(2.0)
      .into_iter()
      .map(|mut point| {
        point.x = point.x.min(50.0);
        point
      })
      .collect();
    let mut water = vec![false; (size * size) as usize];
    let mut lakes = water.clone();

    for x in 4..=50 {
      water[(20 * size + x) as usize] = true;
    }

    for y in 40..=44 {
      for x in 10..30 {
        water[(y * size + x) as usize] = true;
        lakes[(y * size + x) as usize] = true;
      }
    }

    let seeds = wet_bank_seeds(&map, &water, &lakes, &[brook], &[]);
    let wet = WetBanks::from_seeds(&map, &seeds, &lakes);
    let near = |d: f32, expected: f32| (d - expected).abs() <= 0.2;

    // On the brook, and one sample (30 m) from its centreline: 29 m from
    // its edge, not the 15 m a sample-wide stream would give.
    assert_eq!(wet.at(30, 20).0, 0.0);
    assert!(near(wet.at(30, 21).0, 29.0), "{:?}", wet.at(30, 21));
    assert!(wet.at(30, 22).0 >= WET_BANK_RANGE_METRES - 0.2);
    // A lake fills its samples, so its edge lies half a sample out.
    assert_eq!(wet.at(20, 42).0, 0.0);
    assert!(near(wet.at(20, 45).0, 15.0), "{:?}", wet.at(20, 45));
    assert!(wet.at(20, 46).0 >= WET_BANK_RANGE_METRES - 0.2);
    // Matching `WetBanks::build` for plain masks.
    assert_eq!(
      WetBanks::build(&map, &lakes, &lakes).distance[..],
      {
        let seeds: Vec<f32> = lakes
          .iter()
          .map(|lake| if *lake { -15.0 } else { f32::MAX })
          .collect();
        WetBanks::from_seeds(&map, &seeds, &lakes).distance
      }[..]
    );
  }

  #[test]
  fn riparian_greening_is_full_at_the_water_and_ends_at_its_reach() {
    let map = HeightMap::flat(
      200,
      60,
      10.0,
      TerrainMetadata {
        width: 200,
        height: 60,
        metres_per_sample: 10.0,
        ..TerrainMetadata::default()
      },
    );
    let dry = vec![false; 200 * 60];
    // Along row 20; 1 m³/s reaches R = 25 + 12 = 37 m.
    let river = |discharge: f32| {
      let points = straight_reach(200)
        .into_iter()
        .map(|mut point| {
          point.discharge = discharge;
          point
        })
        .collect();
      vec![Reach {
        points,
        ..Reach::default()
      }]
    };
    let at = |field: &[u8], dy: usize| field[(20 + dy) * 200 + 50];
    let field = riparian_field(&map, &river(1.0), &dry, 1.0);

    assert_eq!(at(&field, 0), 255);
    // (1 - 30 / 37)^2 = 0.036.
    assert!((8..=10).contains(&at(&field, 3)), "{}", at(&field, 3));
    assert_eq!(at(&field, 4), 0);
    // Bigger rivers reach further: 100 m³/s reaches 145 m.
    assert!(at(&riparian_field(&map, &river(100.0), &dry, 1.0), 12) > 0);
    // A strength of 2 doubles it, up to 1.
    assert_eq!(at(&riparian_field(&map, &river(1.0), &dry, 2.0), 1), 255);
    // Lakes reach 40 m.
    let mut lake = dry.clone();
    lake[20 * 200 + 50] = true;
    let field = riparian_field(&map, &[], &lake, 1.0);
    assert!(at(&field, 3) > 10 && at(&field, 4) == 0);
    assert!(riparian_field(&map, &river(1.0), &dry, 0.0).is_empty());
    assert!(riparian_field(&map, &[], &dry, 1.0).is_empty());
  }

  #[test]
  fn river_mouths_near_sea_level_are_sand() {
    let (mut map, mut reaches, mask) = bed_scene(1.5, 0.0);
    map.metadata.sea_level_metres = 49.8;
    for point in &mut reaches[0].points {
      point.level = 50.0;
    }
    let bed = bed_materials(&map, &reaches, &[], &mask);
    let beside = bed_at(&bed, 30, 21);
    assert!(beside[1] > beside[0], "{beside:?}");
  }

  #[test]
  fn simplifying_a_straight_reach_drops_most_of_its_points() {
    let points = straight_reach(400);
    let kept = simplify_ribbon(&points, &[true; 400], 12.0, &|_| false);

    assert!(
      kept.len() as f32 <= points.len() as f32 * 0.6,
      "kept {} of {}",
      kept.len(),
      points.len()
    );
    assert_eq!(kept.first(), points.first());
    assert_eq!(kept.last(), points.last());
  }

  #[test]
  fn simplifying_keeps_joins_falls_ends_and_bends() {
    let mut points = straight_reach(200);
    points[120].falling = true;
    points[121].falling = true;
    points[60].curvature = 0.5;
    let join = [points[90].x, points[90].y];
    let kept = simplify_ribbon(&points, &[true; 200], 12.0, &|point| {
      point.x == join[0] && point.y == join[1]
    });
    let has = |i: usize| kept.iter().any(|point| *point == points[i]);

    for i in [0, 60, 90, 119, 120, 121, 122, 199] {
      assert!(has(i), "point {i} was dropped");
    }

    // A sideways kink is never smoothed away.
    let mut kinked = straight_reach(200);
    kinked[100].y += 0.2;
    let kept = simplify_ribbon(&kinked, &[true; 200], 12.0, &|_| false);
    assert!(kept.iter().any(|point| *point == kinked[100]));
  }

  /// A slow, straight stream `width` metres wide on a flat 30 m grid.
  fn flat_brook(width: f32) -> Vec<ChannelPoint> {
    straight_reach(160)
      .into_iter()
      .map(|mut point| {
        point.x = 4.0 + (point.x - 4.0) * 2.0;
        point.width = width;
        point.slope = 0.001;
        point.speed = 0.4;
        point
      })
      .collect()
  }

  /// A brook 1.5 m wide down a 15 % slope on a 2 m grid, its surface
  /// 0.3 m above the ground, settled as the build settles it.
  fn steep_brook() -> (HeightMap, Vec<ChannelPoint>) {
    let mut map = HeightMap::flat(
      64,
      16,
      0.0,
      TerrainMetadata {
        width: 64,
        height: 16,
        metres_per_sample: 2.0,
        sea_level_metres: -100.0,
        ..TerrainMetadata::default()
      },
    );

    for (i, height) in map.heights.iter_mut().enumerate() {
      *height = 100.0 - 0.3 * (i % 64) as f32;
    }

    let mut points: Vec<ChannelPoint> = (0..224)
      .map(|i| {
        let x = 4.0 + i as f32 * 0.25;
        ChannelPoint {
          x,
          y: 8.0,
          level: 100.3 - 0.3 * x,
          bed: 100.0 - 0.3 * x,
          width: 1.5,
          depth: 0.3,
          slope: 0.15,
          speed: 1.5,
          ..straight_reach(1)[0]
        }
      })
      .collect();
    settle_on_ground(&mut points, &map);
    (map, points)
  }

  #[test]
  fn steep_streams_step_from_level_pool_to_level_pool() {
    let (map, mut points) = steep_brook();
    let (first, last) = (points[0].level, points[points.len() - 1].level);
    let into = step_pools(&mut points, &map, (2.0, [64.0, 16.0]), 7, &|_| false);
    assert_eq!(into.len(), points.len());
    let arcs = crate::terrain::channels::arc_lengths(
      &points.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
      2.0,
    );
    // Lips: where a pool ends and the next begins, a row just above the
    // drop and one just below it.
    let mut lips = Vec::new();

    for k in 1..points.len() {
      assert!(
        points[k].level <= points[k - 1].level,
        "water climbs at {k}"
      );

      if into[k - 1] > 0.0 && into[k] > 0.0 {
        if into[k] < into[k - 1] {
          assert!(
            points[k].level < points[k - 1].level - 0.1,
            "no drop at the lip at {k}"
          );
          assert!(arcs[k] - arcs[k - 1] <= STEP_FACE_METRES + 1e-3);
          lips.push(arcs[k - 1]);
        } else {
          // Level, or held up over the ground.
          let floor = full_detail_height(&map, points[k - 1].x, points[k - 1].y) + 0.4 * 0.3;
          assert!(
            points[k].level == points[k - 1].level || (points[k - 1].level - floor).abs() < 1e-4,
            "a pool slopes at {k}"
          );
        }
      }
    }

    assert!(lips.len() >= 25, "{} lips", lips.len());

    for pair in lips.windows(2) {
      let spacing = pair[1] - pair[0];
      assert!(
        (1.5 * 1.5..=3.0 * 1.5).contains(&spacing),
        "steps {spacing} m apart"
      );
    }

    // The stream falls as far as before, never stands more than half a
    // metre over its depth above the ground, and never above the smooth
    // surface.
    let drop = points[0].level - points[points.len() - 1].level;
    assert!(
      (drop - (first - last)).abs() < 1e-3,
      "drop {drop} against {}",
      first - last
    );

    for p in &points {
      assert!(p.level <= full_detail_height(&map, p.x, p.y) + p.depth + 0.5 + 1e-4);
      // Below the smooth surface, so it never stands proud of its banks,
      // but never sunk into the ground.
      assert!(
        p.level <= 100.3 - 0.3 * p.x + 1e-3,
        "{} at {}",
        p.level,
        p.x
      );
      assert!(
        p.level >= 100.0 - 0.3 * p.x + 0.4 * 0.3 - 1e-3,
        "{} at {}",
        p.level,
        p.x
      );
    }
  }

  #[test]
  fn gentle_streams_cascades_and_the_ends_of_steep_ones_do_not_step() {
    // Gentle streams run in riffles and pools, and cascades tumble.
    for slope in [0.04, 0.5] {
      let (map, mut points) = steep_brook();

      for p in &mut points {
        p.slope = slope;
      }

      let before = points.clone();
      let into = step_pools(&mut points, &map, (2.0, [64.0, 16.0]), 7, &|_| false);
      assert!(into.iter().all(|t| *t == 0.0));
      assert_eq!(points, before);
    }

    // Steps stop two widths short of a join.
    let (map, mut points) = steep_brook();
    let join = points[100];
    let into = step_pools(&mut points, &map, (2.0, [64.0, 16.0]), 7, &|p| *p == join);
    let at = points
      .iter()
      .position(|p| *p == join)
      .expect("the join stays");
    let arcs = crate::terrain::channels::arc_lengths(
      &points.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
      2.0,
    );

    for (k, t) in into.iter().enumerate() {
      if (arcs[k] - arcs[at]).abs() <= 3.0 {
        assert_eq!(*t, 0.0, "a step {} m from the join", arcs[k] - arcs[at]);
      }
    }
  }

  #[test]
  fn rivers_take_their_colour_from_their_catchment() {
    // Three rows of ten flow cells, each draining east: the first fed by
    // two cells of glacier ice at its head, the second through bog all
    // the way, the third over plain ground.
    let (width, height) = (10u32, 3u32);
    let cells = (width * height) as usize;
    let hydrology = Hydrology {
      width,
      height,
      stride: 1,
      receiver: (0..cells as u32)
        .map(|cell| {
          if cell % width == width - 1 {
            NO_RECEIVER
          } else {
            cell + 1
          }
        })
        .collect(),
      order: (0..width)
        .rev()
        .flat_map(|x| (0..height).map(move |y| y * width + x))
        .collect(),
      glacier: (0..cells as u32)
        .map(|cell| cell / width == 0 && cell % width < 2)
        .collect(),
      ..Hydrology::default()
    };
    let surface: Vec<SurfaceSample> = (0..cells)
      .map(|index| SurfaceSample {
        biome: if index / width as usize == 1 {
          vista_types::BiomeKind::SwampWetlands as u8
        } else {
          vista_types::BiomeKind::GrassyMeadows as u8
        },
        celsius_hundredths: 1200,
        ..SurfaceSample::default()
      })
      .collect();
    let area =
      crate::terrain::drainage::accumulate(&hydrology.order, &hydrology.receiver, vec![1.0; cells]);
    let shares = catchment_shares(&hydrology, &area, &surface, width);
    let mouth = |row: u32| catchment_code(shares[(row * width + width - 1) as usize]);
    // Glacial flour below the snout, peat from the bog, clear otherwise.
    assert_eq!(mouth(0), 8.0 * 7.0);
    assert_eq!(mouth(1), 7.0);
    assert_eq!(mouth(2), 0.0);

    // The code rides on the swirl and comes back whole, as the shader
    // reads it.
    for code in [0.0, 7.0, 56.0, 63.0] {
      for swirl in [-1.0, -0.3, 0.0, 0.999] {
        let carried: f32 = swirl + 4.0 * code;
        assert_eq!(((carried + 2.0) * 0.25).floor(), code);
      }
    }

    assert!(
      include_str!("../shaders/water.wgsl").contains("let code = floor((in.swirl + 2.0) * 0.25);")
    );
  }

  #[test]
  fn a_plume_spreads_seawards_and_fades_to_the_sea_within_its_length() {
    let (width, length) = (60.0, plume_length(60.0, 300.0));
    assert!((5.0 * width..=20.0 * width).contains(&length), "{length} m");
    assert!((plume_length(10.0, 1.0) - 50.0).abs() < 1e-3);
    assert!((plume_length(10.0, 5000.0) - 200.0).abs() < 1e-3);
    // Full at the mouth, gone by its length and beyond its spread.
    assert!(plume_share(0.0, 0.0, width, length) > 0.99);
    assert_eq!(plume_share(length, 0.0, width, length), 0.0);
    assert_eq!(plume_share(0.5 * length, 2.0 * width, width, length), 0.0);
    assert_eq!(plume_share(-width, 0.0, width, length), 0.0);
    // Wider seawards: halfway out, more than twice the mouth's width still
    // shows it.
    assert!(plume_share(0.5 * length, 1.2 * width, width, length) > 0.0);
    assert_eq!(plume_share(0.0, 1.2 * width, width, length), 0.0);
    let along: Vec<f32> = (0..=20)
      .map(|k| plume_share(k as f32 * 0.05 * length, 0.0, width, length))
      .collect();
    assert!(along.windows(2).all(|pair| pair[1] <= pair[0] + 1e-6));

    // The frame takes the rivers that matter most from the camera.
    let mouth = |x: f32, discharge: f32| SeaMouth {
      at: [x, 0.0],
      toward: [0.0, -1.0],
      width: 40.0,
      discharge,
      code: 3.0,
    };
    let mouths: Vec<SeaMouth> = (0..12)
      .map(|k| mouth(k as f32 * 1000.0, 10.0 + k as f32))
      .chain([mouth(50_000.0, 5000.0)])
      .collect();
    let words = plume_mouths(&mouths, [0.0, 0.0]);
    assert_eq!(words[0][0], 50_000.0);
    assert!(words
      .chunks(2)
      .all(|pair| pair[1][0] == 40.0 && pair[1][2] == 3.0));
    assert!(plume_mouths(&[], [0.0, 0.0])
      .iter()
      .all(|word| word[0] == 0.0));
    let wgsl = include_str!("../shaders/water.wgsl");
    assert!(wgsl.contains("let spread = 0.5 * width * (1.0 + 3.0 * saturate(along / length));"));
  }

  /// A loop budget that never runs out.
  fn unlimited() -> usize {
    usize::MAX
  }

  #[test]
  fn steep_narrow_streams_wander_within_their_corridor() {
    let table = kinoshita_table();
    let carved: Vec<ChannelPoint> = flat_brook(1.5)
      .into_iter()
      .map(|mut point| {
        point.slope = 0.15;
        point.speed = 1.5;
        point
      })
      .collect();
    let centre = sub_sample_centreline(
      &carved,
      12.0,
      1.0,
      0.3,
      &table,
      &|_| false,
      &mut unlimited(),
    );
    let y = carved[0].y;
    let off: Vec<f32> = centre.iter().map(|p| (p.y - y).abs()).collect();
    // Within the corridor, but off the straight line by metres somewhere.
    assert!(off.iter().all(|d| *d <= CORRIDOR_SAMPLES + 1e-4));
    let most = off.iter().copied().fold(0.0f32, f32::max) * 12.0;
    assert!(most >= 2.0, "{most} m at most");
    // Gently: a wander, not a meander.
    let length: f32 = centre
      .windows(2)
      .map(|pair| length2(pair[1].x - pair[0].x, pair[1].y - pair[0].y))
      .sum();
    let sinuosity = length / (centre[centre.len() - 1].x - centre[0].x);
    assert!((1.01..1.25).contains(&sinuosity), "sinuosity {sinuosity}");
    assert_eq!(centre[0], carved[0]);
  }

  #[test]
  fn narrow_stream_loops_share_a_budget() {
    let table = kinoshita_table();
    let carved = flat_brook(2.0);
    let mut budget = usize::MAX;
    let looped = sub_sample_centreline(&carved, 30.0, 1.0, 0.3, &table, &|_| false, &mut budget);
    let used = usize::MAX - budget;
    assert!(looped.len() > carved.len() && used >= looped.len() - 1);

    // Too little left: the stream keeps its carved path, and spends none.
    let mut short = used - 1;
    let kept = sub_sample_centreline(&carved, 30.0, 1.0, 0.3, &table, &|_| false, &mut short);
    assert_eq!(kept, carved);
    assert_eq!(short, used - 1);
  }

  #[test]
  fn narrow_stream_loops_vary_in_length() {
    let table = kinoshita_table();
    let carved = flat_brook(2.0);
    let centre = sub_sample_centreline(
      &carved,
      30.0,
      1.0,
      0.3,
      &table,
      &|_| false,
      &mut unlimited(),
    );
    // Where the loop crosses the carved path going one way: one per
    // wavelength, about 22 m for a 2 m stream.
    let crossings: Vec<f32> = centre
      .windows(2)
      .filter(|pair| pair[0].y < 20.0 && pair[1].y >= 20.0)
      .map(|pair| {
        let t = (20.0 - pair[0].y) / (pair[1].y - pair[0].y);
        (pair[0].x + (pair[1].x - pair[0].x) * t) * 30.0
      })
      .collect();
    let lengths: Vec<f32> = crossings.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let shortest = lengths.iter().copied().fold(f32::MAX, f32::min);
    let longest = lengths.iter().copied().fold(0.0, f32::max);
    assert!(lengths.len() > 100, "{} loops", lengths.len());
    assert!(
      shortest > 12.0 && longest < 40.0,
      "{shortest} to {longest} m"
    );
    assert!(longest > shortest * 1.4, "{shortest} to {longest} m");
  }

  #[test]
  fn narrow_stream_loops_pass_through_joins() {
    let table = kinoshita_table();
    let carved = flat_brook(2.0);
    let join = carved[80];
    let centre = sub_sample_centreline(
      &carved,
      30.0,
      1.0,
      0.3,
      &table,
      &|point| *point == join,
      &mut unlimited(),
    );
    let at_join = centre
      .iter()
      .min_by(|a, b| (a.x - join.x).abs().total_cmp(&(b.x - join.x).abs()))
      .unwrap();

    assert!(
      (at_join.y - join.y).abs() < 1e-3,
      "{} samples off the join",
      (at_join.y - join.y).abs()
    );
    assert!(centre.iter().any(|point| (point.y - join.y).abs() > 0.1));
  }

  #[test]
  fn narrow_streams_meander_within_their_carved_corridor() {
    let table = kinoshita_table();
    let carved = flat_brook(2.0);
    let centre = sub_sample_centreline(
      &carved,
      30.0,
      1.0,
      0.3,
      &table,
      &|_| false,
      &mut unlimited(),
    );
    let y = carved[0].y;

    for point in &centre {
      assert!(
        (point.y - y).abs() <= CORRIDOR_SAMPLES + 1e-4,
        "{} samples off the carved path",
        (point.y - y).abs()
      );
    }

    let length: f32 = centre
      .windows(2)
      .map(|pair| length2(pair[1].x - pair[0].x, pair[1].y - pair[0].y))
      .sum();
    let chord = centre[centre.len() - 1].x - centre[0].x;
    assert!(length / chord >= 1.3, "sinuosity {}", length / chord);
    assert_eq!(centre[0], carved[0]);

    // Streams a sample wide or more keep their carved path.
    let wide = flat_brook(40.0);
    assert_eq!(
      sub_sample_centreline(&wide, 30.0, 1.0, 0.3, &table, &|_| false, &mut unlimited()),
      wide
    );
    assert_eq!(
      sub_sample_centreline(
        &carved,
        30.0,
        0.0,
        0.3,
        &table,
        &|_| false,
        &mut unlimited()
      ),
      carved
    );
  }

  /// The banks `points` get on `map`, with no joins.
  fn banks_of(map: &HeightMap, points: &[ChannelPoint]) -> RiverNetwork {
    let metres = map.metadata.metres_per_sample;
    let arcs = crate::terrain::channels::arc_lengths(
      &points.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
      metres,
    );
    let mut network = RiverNetwork::default();
    let plan = plan_bank_strips(map, points, &arcs, metres, 7);
    add_bank_strips(
      &mut network,
      map,
      points,
      (&plan, &arcs),
      metres,
      [1905.0, 945.0],
    );
    network
  }

  #[test]
  fn only_streams_narrower_than_a_sample_get_banks() {
    let map = HeightMap::flat(
      128,
      64,
      5.0,
      TerrainMetadata {
        width: 128,
        height: 64,
        metres_per_sample: 30.0,
        ..TerrainMetadata::default()
      },
    );
    let low: Vec<_> = flat_brook(2.0)
      .into_iter()
      .map(|mut point| {
        point.level = 4.8;
        point
      })
      .collect();
    let network = banks_of(&map, &low);
    assert!(!network.bank_vertices.is_empty());
    assert_eq!(network.bank_vertices.len() % BANK_PROFILE, 0);
    assert_eq!(network.bank_indices.len() % 6, 0);

    for row in network.bank_vertices.chunks_exact(BANK_PROFILE) {
      // From the water's edge, just under the water, out to the turf's
      // back on the drawn ground.
      assert!(row[0].position[1] < 4.8 && row[1].position[1] > 4.8);
      assert!((row[4].position[1] - 5.02).abs() < 1e-4);
      let profile: Vec<f32> = row.iter().map(|v| v.params[0]).collect();
      assert_eq!(profile, [0.0, 1.0, 2.0, 3.0, 4.0]);
    }

    // The turf's back is never below the water surface of a stream over a
    // coarse trench.
    let high: Vec<_> = flat_brook(2.0)
      .into_iter()
      .map(|mut point| {
        point.level = 5.5;
        point
      })
      .collect();
    let raised = banks_of(&map, &high);
    assert!(raised
      .bank_vertices
      .chunks_exact(BANK_PROFILE)
      .all(|row| (row[4].position[1] - 5.55).abs() < 1e-4));

    assert!(banks_of(&map, &flat_brook(40.0)).bank_vertices.is_empty());
  }

  #[test]
  fn cut_banks_stand_steep_and_inner_banks_shelve() {
    let mut p = flat_brook(2.0)[10];
    p.curvature = 0.5;

    for depth in [0.1, 0.4, 1.0, 3.0] {
      p.depth = depth;

      for along in (0..200).map(|k| k as f32 * 0.7) {
        // The outer bank of a left turn is on the right.
        let cut = bank_shape(&p, -1.0, along, (0.0, 2.0), 3);
        assert!(cut.cut > 0.99);
        assert!((0.2..=1.5).contains(&cut.face), "a {} m face", cut.face);
        assert!(
          (0.1..=0.4).contains(&(cut.overhang + 1e-6)),
          "{}",
          cut.overhang
        );
        let [_, foot, top, lip] = cut.points();
        let steep = ((top[1] - foot[1]) / (top[0] - foot[0]))
          .atan()
          .to_degrees();
        assert!(steep > 60.0, "a {steep} degree face");
        // The lip hangs out over the face's foot side.
        assert!(lip[0] < top[0] && lip[1] > top[1]);

        let inner = bank_shape(&p, 1.0, along, (0.0, 2.0), 3);
        assert_eq!(inner.cut, 0.0);
        let [edge, _, _, lip] = inner.points();
        let slope = ((lip[1] - edge[1]) / (lip[0] - edge[0]))
          .atan()
          .to_degrees();
        assert!(slope < 20.0, "a {slope} degree shelf");
      }
    }

    // A steep valley side is cut whatever the bend.
    p.curvature = 0.0;
    assert!(bank_shape(&p, 1.0, 5.0, (0.9, 2.0), 3).cut > 0.99);
    assert_eq!(bank_shape(&p, 1.0, 5.0, (0.1, 2.0), 3).cut, 0.0);
    // Ground barely above the water has no face, and no bank stands above
    // the ground beside it.
    assert_eq!(bank_shape(&p, 1.0, 5.0, (0.9, 0.1), 3).cut, 0.0);
    assert!(bank_shape(&p, 1.0, 5.0, (0.9, 0.35), 3).face <= 0.45);
  }

  #[test]
  fn a_tributarys_banks_take_the_main_stems_at_the_join() {
    let map = HeightMap::flat(
      64,
      64,
      5.0,
      TerrainMetadata {
        width: 64,
        height: 64,
        metres_per_sample: 12.0,
        ..TerrainMetadata::default()
      },
    );
    let points = flat_brook(1.5);
    let arcs = crate::terrain::channels::arc_lengths(
      &points.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
      12.0,
    );
    let mut strips = plan_bank_strips(&map, &points, &arcs, 12.0, 9);
    let main = BankShape {
      cut: 1.0,
      margin: 0.15,
      face: 1.2,
      run: 0.12,
      overhang: 0.3,
      lip: 0.25,
    };
    blend_join(&mut strips, &arcs, 1.5, main);
    let end = *arcs.last().unwrap();

    for row in &strips.rows {
      let left = end - arcs[row.point as usize];
      let [own, theirs] = [row.shape.points(), main.points()];
      let apart = own
        .iter()
        .zip(&theirs)
        .map(|(a, b)| length2(a[0] - b[0], a[1] - b[1]))
        .fold(0.0f32, f32::max);

      // Along the blend's last tenth, within 5 cm of the main stem's.
      if left < 0.3 {
        assert!(apart < 0.05, "{apart} m apart {left} m from the join");
      }

      // Beyond 2 w, its own.
      if left > 3.0 {
        assert!(apart > 0.05 || row.shape.cut > 0.9);
      }
    }
  }

  #[test]
  fn reeds_line_the_true_banks_of_slow_brooks_on_coarse_maps() {
    use crate::render::flora::GRASS_STYLE_REED;
    use crate::render::grass::build_reed_instances;
    // A very gentle valley on a 120 m grid: slow brooks far narrower than
    // a sample.
    let size = 64;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 120.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 0.0, metadata);

    for y in 0..size {
      for x in 0..size {
        let _ = map.set_height(x, y, y as f32 * 0.06 + 2.0 + (x as f32 - 32.0).abs() * 0.5);
      }
    }

    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    let options = RiverOptions {
      min_catchment_km2: 2.0,
      inflow: vista_types::RiverInflows::Mode(vista_types::InflowMode::None),
      ..RiverOptions::default()
    };
    let sources = plain(map.heights.len());
    let network = build_river_network(&mut map, &options, sources);
    assert!(!network.brooks.is_empty(), "no slow brook");
    let warm = vec![
      SurfaceSample {
        celsius_hundredths: 1500,
        ..SurfaceSample::default()
      };
      map.heights.len()
    ];
    let grass = vista_types::GrassOptions {
      enabled: true,
      density: 1.0,
      ..vista_types::GrassOptions::default()
    };
    let instances = build_reed_instances(
      &map,
      &warm,
      Some(&network.wet),
      &network.brooks,
      &grass,
      1.0,
    );
    let reeds: Vec<_> = instances
      .iter()
      .filter(|instance| instance.style == GRASS_STYLE_REED)
      .collect();
    assert!(reeds.len() > 20, "{} reeds", reeds.len());

    for reed in reeds {
      let [x, _, z] = reed.position;
      let edge = network
        .brooks
        .iter()
        .flat_map(|run| run.windows(2))
        .map(|pair| {
          let (a, b) = (pair[0], pair[1]);
          let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
          let t =
            (((x - a[0]) * dx + (z - a[1]) * dz) / (dx * dx + dz * dz).max(1e-6)).clamp(0.0, 1.0);
          length2(x - a[0] - dx * t, z - a[1] - dz * t) - a[2].max(b[2])
        })
        .fold(f32::MAX, f32::min);
      assert!(edge <= 3.05, "a reed {edge} m from the water's edge");
    }
  }

  /// A temperate valley 1.5 km long at 12 m, falling `fall` along it, with
  /// a brook meandering down its floor, and its river network.
  fn meander_valley(fall: f32) -> (HeightMap, RiverNetwork) {
    let size = 128;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 12.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 0.0, metadata);

    for y in 0..size {
      for x in 0..size {
        let (wx, wz) = ((x as f32 - 64.0) * 12.0, (y as f32 - 64.0) * 12.0);
        let centre = 90.0 * (wz / 160.0).sin();
        let height = 30.0 + (size - y) as f32 * 12.0 * fall + (wx - centre).abs() * 0.04;
        let _ = map.set_height(x, y, height);
      }
    }

    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    let options = RiverOptions {
      min_catchment_km2: 0.5,
      inflow: vista_types::RiverInflows::Mode(vista_types::InflowMode::None),
      ..RiverOptions::default()
    };
    let sources = plain(map.heights.len());
    let network = build_river_network(&mut map, &options, sources);
    (map, network)
  }

  #[test]
  fn the_ribbon_covers_an_inflow_channel_right_to_the_map_edge() {
    // A valley runs in from the south edge, where the automatic inflow
    // enters, and drains north to the sea.
    let mut map = crate::terrain::hydrology::tests::map_from(96, 30.0, |x, y| {
      y as f32 * 0.5 - 3.0 + (x as f32 - 40.0).abs() * 0.8
    });
    let sources = plain(map.heights.len());
    let network = build_river_network(&mut map, &RiverOptions::default(), sources);
    assert_eq!(network.inflows.len(), 1);
    let inflow = network.inflows[0].position;
    assert_eq!(inflow, [40.0, 95.0]);
    let reach = network
      .reaches
      .iter()
      .find(|reach| {
        reach
          .points
          .first()
          .is_some_and(|p| p.x == inflow[0] && p.y == inflow[1])
      })
      .expect("a reach starts at the inflow");
    let half = [95.0 * 30.0 * 0.5; 2];
    // Whether world `(x, z)` lies in a river triangle of the ribbon.
    let covered = |x: f32, z: f32| {
      network.indices.chunks(3).any(|triangle| {
        let corner = |i: usize| network.vertices[triangle[i] as usize];

        if corner(0).kind() != WATER_KIND_RIVER {
          return false;
        }

        let [a, b, c] = [0, 1, 2].map(|i| [corner(i).position[0], corner(i).position[2]]);
        let side =
          |p: [f32; 2], q: [f32; 2]| (q[0] - p[0]) * (z - p[1]) - (q[1] - p[1]) * (x - p[0]);
        let (d0, d1, d2) = (side(a, b), side(b, c), side(c, a));
        (d0 >= -1e-3 && d1 >= -1e-3 && d2 >= -1e-3) || (d0 <= 1e-3 && d1 <= 1e-3 && d2 <= 1e-3)
      })
    };

    // The inflow's own sample, at the map's edge, is under water, and so
    // is every channel sample inwards, or the ground beside it within the
    // corridor the drawn stream wanders in (the stream runs north, so
    // across is x).
    let [x, z] = to_world([inflow[0], inflow[1]], 30.0, half);
    assert!(covered(x, z), "no water at the inflow");
    let corridor = CORRIDOR_SAMPLES * 30.0;

    for point in &reach.points {
      let [x, z] = to_world([point.x, point.y], 30.0, half);
      let near = (-20..=20).any(|k| covered(x + corridor * k as f32 / 20.0, z));
      assert!(near, "no water by sample {}, {}", point.x, point.y);
    }
  }

  #[test]
  fn a_slow_warm_brook_grows_reeds_along_both_banks() {
    use crate::render::grass::build_reed_instances;
    let grass = vista_types::GrassOptions {
      enabled: true,
      density: 0.5,
      ..vista_types::GrassOptions::default()
    };
    let at = |celsius: f32, map: &HeightMap| {
      vec![
        SurfaceSample {
          celsius_hundredths: (celsius * 100.0) as i16,
          ..SurfaceSample::default()
        };
        map.heights.len()
      ]
    };
    // A 0.1 % fall: a slow brook, under 0.6 m/s.
    let (map, network) = meander_valley(0.001);
    assert!(!network.brooks.is_empty(), "no slow brook");
    let reeds = build_reed_instances(
      &map,
      &at(12.0, &map),
      Some(&network.wet),
      &network.brooks,
      &grass,
      1.0,
    );
    // Which side of the nearest brook each reed stands on.
    let side = |x: f32, z: f32| {
      let mut best = (f32::MAX, 0.0f32);

      for pair in network.brooks.iter().flat_map(|run| run.windows(2)) {
        let (a, b) = (pair[0], pair[1]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let t =
          (((x - a[0]) * dx + (z - a[1]) * dz) / (dx * dx + dz * dz).max(1e-6)).clamp(0.0, 1.0);
        let distance = length2(x - a[0] - dx * t, z - a[1] - dz * t);

        if distance < best.0 {
          best = (distance, dx * (z - a[1]) - dz * (x - a[0]));
        }
      }

      best.1
    };
    let left = reeds
      .iter()
      .filter(|reed| side(reed.position[0], reed.position[2]) > 0.0)
      .count();
    let right = reeds
      .iter()
      .filter(|reed| side(reed.position[0], reed.position[2]) < 0.0)
      .count();
    assert!(left > 20 && right > 20, "{left} left, {right} right");

    // Not in the cold.
    assert!(build_reed_instances(
      &map,
      &at(3.0, &map),
      Some(&network.wet),
      &network.brooks,
      &grass,
      1.0
    )
    .is_empty());

    // Not by fast water: a 4 % fall.
    let (steep, fast) = meander_valley(0.04);
    assert!(fast.brooks.is_empty());
    assert!(build_reed_instances(
      &steep,
      &at(12.0, &steep),
      Some(&fast.wet),
      &fast.brooks,
      &grass,
      1.0
    )
    .is_empty());
  }

  #[test]
  fn plane_sits_at_the_requested_sea_level() {
    let plane = build_water_plane(100.0, 50.0, 12.5);

    assert!(plane.iter().all(|vertex| vertex.position[1] == 12.5));
  }

  #[test]
  fn plane_spans_the_requested_footprint() {
    let plane = build_water_plane(100.0, 50.0, 0.0);
    let xs: Vec<f32> = plane.iter().map(|vertex| vertex.position[0]).collect();
    let zs: Vec<f32> = plane.iter().map(|vertex| vertex.position[2]).collect();

    assert!(xs.contains(&-100.0) && xs.contains(&100.0));
    assert!(zs.contains(&-50.0) && zs.contains(&50.0));
  }

  #[test]
  fn ocean_grid_is_fine_near_the_centre_and_reaches_the_horizon() {
    let (vertices, indices) = build_ocean_grid(129, 50_000.0);

    assert_eq!(vertices.len(), 129 * 129);
    assert_eq!(indices.len(), 128 * 128 * 6);
    let centre = &vertices[64 * 129 + 64];
    assert_eq!(centre.position[0], 0.0);
    assert!(centre.params[2] <= OCEAN_BASE_SPACING_METRES * 1.01);
    assert!(vertices.iter().any(|vertex| vertex.position[0] >= 50_000.0));
  }

  #[test]
  fn ocean_band_spacing_divides_the_snap_distance() {
    // Grid vertices inside the displaced region must stay put when the
    // grid origin jumps by `OCEAN_SNAP_METRES`.
    let mut step = OCEAN_BASE_SPACING_METRES;

    for _ in 0..6 {
      assert!((OCEAN_SNAP_METRES / step).fract().abs() < 1e-4);
      step *= 2.0;
    }
  }

  fn plain(samples: usize) -> RiverSources<'static> {
    RiverSources {
      surface: &[],
      seed: 1,
      painted: Vec::new(),
      record: CarveRecord::new(samples),
    }
  }

  fn valley_map() -> HeightMap {
    let size = 96;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 40.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 0.0, metadata);

    // A V-shaped valley draining north to the sea.
    for y in 0..size {
      for x in 0..size {
        let across = (x as f32 - 48.0).abs() * 6.0;
        let along = y as f32 * 4.0 - 20.0;
        let _ = map.set_height(x, y, along + across);
      }
    }

    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    map
  }

  #[test]
  fn plunge_pools_on_a_slope_lie_on_the_ground() {
    // A 20 m cliff above a hillside falling at about 30 degrees, in a
    // valley that gathers the water.
    let size = 128;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 2.0,
      sea_level_metres: -100.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 0.0, metadata);

    for y in 0..size {
      for x in 0..size {
        let step = if y >= 64 { 20.0 } else { 0.0 };
        let _ = map.set_height(x, y, y as f32 * 1.2 + (x as f32 - 64.0).abs() * 0.5 + step);
      }
    }

    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    // A river entering at the top of the valley: the valley alone drains
    // only a trickle, which has no pool.
    let options = RiverOptions {
      min_catchment_km2: 0.005,
      inflow: vista_types::RiverInflows::List(vec![vista_types::RiverInflow {
        position: [0.0, 120.0],
        discharge_cubic_metres_per_second: 4.0,
      }]),
      ..RiverOptions::default()
    };
    let sources = plain(map.heights.len());
    let network = build_river_network(&mut map, &options, sources);
    let half = (size as f32 - 1.0) * 2.0 * 0.5;
    let pools: Vec<&WaterVertex> = network
      .vertices
      .iter()
      .filter(|vertex| vertex.kind() == WATER_KIND_POOL)
      .collect();

    assert!(!network.falls.is_empty());
    assert!(!pools.is_empty());

    // No part of a pool stands clear of the ground: at most a film over
    // it, or, towards the outlet, the depth of the stream leaving it.
    let outlet = network
      .falls
      .iter()
      .map(|fall| channel_depth(fall.discharge))
      .fold(0.0, f32::max);

    for vertex in pools {
      let x = (vertex.position[0] + half) / 2.0;
      let y = (vertex.position[2] + half) / 2.0;
      let ground = height_at(&map, x, y).max(full_detail_height(&map, x, y));
      let above = vertex.position[1] - ground;
      let allowed = POOL_FILM_METRES + outlet + 0.03;

      assert!(
        above <= allowed,
        "pool {above} m above the ground at {x}, {y}"
      );
    }
  }

  #[test]
  fn a_trickle_fall_has_no_sheet_mist_or_pool() {
    let size = 128;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 2.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 0.0, metadata);

    for y in 0..size {
      for x in 0..size {
        let step = if y >= 64 { 20.0 } else { 0.0 };
        let _ = map.set_height(
          x,
          y,
          y as f32 * 0.1 - 0.5 + (x as f32 - 64.0).abs() * 0.5 + step,
        );
      }
    }

    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    let options = RiverOptions {
      min_catchment_km2: 0.005,
      inflow: vista_types::RiverInflows::Mode(vista_types::InflowMode::None),
      ..RiverOptions::default()
    };
    let sources = plain(map.heights.len());
    let network = build_river_network(&mut map, &options, sources);

    assert!(!network.falls.is_empty());
    assert!(network.falls.iter().all(|fall| fall.trickle));
    assert!(network.fall_vertices.is_empty());
    assert!(!network
      .vertices
      .iter()
      .any(|vertex| vertex.kind() == WATER_KIND_POOL));
  }

  #[test]
  fn rivers_follow_the_valley_and_are_carved() {
    let mut map = valley_map();
    let before = map.heights.clone();
    let options = RiverOptions {
      min_catchment_km2: 0.5,
      ..RiverOptions::default()
    };
    let sources = plain(map.heights.len());
    let network = build_river_network(&mut map, &options, sources);

    assert!(network.river_count > 0);
    assert!(!network.indices.is_empty());
    assert!(!network.carved.is_empty());
    assert!(network.mask.iter().any(|value| *value));

    // River vertices hug the valley floor.
    let river_x: Vec<f32> = network
      .vertices
      .iter()
      .filter(|vertex| vertex.kind() == WATER_KIND_RIVER)
      .map(|vertex| vertex.position[0])
      .collect();
    let mean_x = river_x.iter().sum::<f32>() / river_x.len() as f32;
    assert!(mean_x.abs() < 400.0, "mean river x {mean_x}");

    // Every river vertex flows north (towards lower y / negative z).
    assert!(network
      .vertices
      .iter()
      .filter(|vertex| vertex.kind() == WATER_KIND_RIVER)
      .all(|vertex| vertex.flow[1] <= 0.01));

    restore_carving(&mut map, &network.carved);
    assert_eq!(map.heights, before);
  }

  #[test]
  fn river_extraction_is_deterministic() {
    let options = RiverOptions {
      min_catchment_km2: 0.5,
      ..RiverOptions::default()
    };
    let mut first_map = valley_map();
    let mut second_map = valley_map();
    let sources = plain(first_map.heights.len());
    let first = build_river_network(&mut first_map, &options, sources);
    let sources = plain(second_map.heights.len());
    let second = build_river_network(&mut second_map, &options, sources);

    assert_eq!(first.vertices, second.vertices);
    assert_eq!(first_map.heights, second_map.heights);
  }

  #[test]
  fn disabled_rivers_leave_the_terrain_untouched() {
    let mut map = valley_map();
    let before = map.heights.clone();
    let options = RiverOptions {
      enabled: false,
      ..RiverOptions::default()
    };
    let sources = plain(map.heights.len());
    let network = build_river_network(&mut map, &options, sources);

    assert!(network.vertices.is_empty());
    assert_eq!(map.heights, before);
  }

  #[test]
  fn closed_basins_become_lakes() {
    let size = 64;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 20.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 100.0, metadata);

    for y in 20..44 {
      for x in 20..44 {
        let _ = map.set_height(x, y, 80.0);
      }
    }

    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    let sources = plain(map.heights.len());
    let network = build_river_network(&mut map, &RiverOptions::default(), sources);

    assert!(network
      .vertices
      .iter()
      .any(|vertex| vertex.kind() == WATER_KIND_LAKE && (vertex.position[1] - 100.0).abs() < 0.5));
  }

  #[test]
  fn straight_strips_merge_rows_and_bends_keep_them() {
    let map = HeightMap::flat(
      128,
      64,
      5.0,
      TerrainMetadata {
        width: 128,
        height: 64,
        metres_per_sample: 30.0,
        ..TerrainMetadata::default()
      },
    );
    // A straight brook with points a quarter of a sample apart.
    let straight: Vec<_> = flat_brook(2.0)
      .into_iter()
      .map(|mut point| {
        point.x = 4.0 + (point.x - 4.0) * 0.25;
        point
      })
      .collect();
    let arcs = vec![0.0; straight.len()];
    let plan = plan_bank_strips(&map, &straight, &arcs, 30.0, 7);
    assert_eq!(plan.runs.len(), 2);

    for run in &plan.runs {
      let run = &plan.rows[run.clone()];
      assert!(run.len() * 3 < straight.len(), "{} rows", run.len());
      // Merged rows are at most a sample apart, so the strip follows the
      // ground.
      for pair in run.windows(2) {
        let (a, b) = (
          &straight[pair[0].point as usize],
          &straight[pair[1].point as usize],
        );
        assert!(length2(b.x - a.x, b.y - a.y) <= 1.0 + 1e-4);
      }
    }

    // A tight bend turns more than 4 degrees at every point: no row goes.
    let bend: Vec<_> = (0..60)
      .map(|i| {
        let angle = i as f32 * 0.2;
        ChannelPoint {
          x: 60.0 + 3.0 * angle.cos(),
          y: 30.0 + 3.0 * angle.sin(),
          ..straight[0]
        }
      })
      .collect();
    let plan = plan_bank_strips(&map, &bend, &arcs, 30.0, 7);
    assert!(plan.runs.iter().all(|run| run.len() >= 58));
  }

  #[test]
  fn the_least_visible_streams_lose_bank_rows_first() {
    let map = HeightMap::flat(
      512,
      512,
      5.0,
      TerrainMetadata {
        width: 512,
        height: 512,
        metres_per_sample: 30.0,
        ..TerrainMetadata::default()
      },
    );
    let stream = |points: u32, width: f32, speed: f32, y: f32| -> Vec<ChannelPoint> {
      (0..points)
        .map(|i| ChannelPoint {
          x: 10.0 + i as f32 * 0.01,
          y,
          width,
          speed,
          ..ChannelPoint::default()
        })
        .collect()
    };
    let strips_along = |points: &[ChannelPoint]| {
      let rows: Vec<BankRow> = (0..points.len() as u32 * 2)
        .map(|point| BankRow {
          point: point / 2,
          side: 1.0,
          outward: [0.0, 1.0],
          shape: BankShape::default(),
          ground: 5.0,
        })
        .collect();
      BankStrips {
        rows,
        runs: vec![0..points.len(), points.len()..points.len() * 2],
        stride: 1,
      }
    };
    // 80,000 bank vertices on a wide, quick stream through the centre,
    // and 400,000 on a narrow, slow one near the edge.
    let drawn = vec![
      stream(8_000, 6.0, 1.0, 255.0),
      stream(40_000, 1.0, 0.2, 10.0),
    ];
    let mut strips: Vec<BankStrips> = drawn.iter().map(|d| strips_along(d)).collect();
    fit_bank_strips(&mut strips, &drawn, &map);
    let total: usize = strips.iter().map(|s| s.vertices(s.stride)).sum();

    assert!(total <= BANK_STRIP_BUDGET, "{total}");
    assert_eq!((strips[0].stride, strips[1].stride), (1, 2));

    // Twice as many narrow streams: the wide one is thinned only after
    // them.
    let drawn = vec![
      stream(8_000, 6.0, 1.0, 255.0),
      stream(40_000, 1.0, 0.2, 10.0),
      stream(40_000, 1.2, 0.2, 20.0),
    ];
    let mut strips: Vec<BankStrips> = drawn.iter().map(|d| strips_along(d)).collect();
    fit_bank_strips(&mut strips, &drawn, &map);
    let total: usize = strips.iter().map(|s| s.vertices(s.stride)).sum();

    assert!(total <= BANK_STRIP_BUDGET, "{total}");
    assert!(strips[0].stride <= strips[1].stride.min(strips[2].stride));
    assert!(strips[1].stride >= 2 && strips[2].stride >= 2);
  }

  /// A gentle coastal plain on a 30 m grid, full of streams narrower
  /// than a sample, whose strips would overrun the budget.
  #[test]
  fn bank_strips_on_a_coastal_plain_stay_within_budget() {
    let size = 512;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 30.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 0.0, metadata);

    for y in 0..size {
      for x in 0..size {
        let (fx, fy) = (x as f32, y as f32);
        let bumps = crate::maths::value_noise(12, fx * 0.03, fy * 0.03)
          + 0.5 * crate::maths::value_noise(13, fx * 0.09, fy * 0.09);
        map.heights[(y * size + x) as usize] = (fy - 8.0) * 30.0 * 0.002 + bumps * 3.0;
      }
    }

    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    let samples = map.heights.len();
    // Enough streams that, drawn along smooth centrelines, their strips
    // would overrun the budget.
    let options = RiverOptions {
      min_catchment_km2: 0.05,
      ..RiverOptions::default()
    };
    let network = build_river_network(&mut map, &options, plain(samples));
    assert!(network.bank_vertices.len() <= BANK_STRIP_BUDGET);
    assert!(network.bank_vertices.len() > BANK_STRIP_BUDGET * 9 / 10);
    assert_eq!(network.bank_indices.len() % 6, 0);
    assert!(network
      .bank_indices
      .iter()
      .all(|index| (*index as usize) < network.bank_vertices.len()));
  }

  /// FNV-1a over bytes, to fingerprint a whole river network.
  fn fingerprint(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
      *hash = (*hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
    }
  }

  /// The mesa desert whose river build is the slowest, with open edges and
  /// its 123 m³/s auto inflow: speeding up the build must not change a
  /// bit of its output. (The hash changed since when bank strips were
  /// fitted into `BANK_STRIP_BUDGET`, when small streams began to seed
  /// the wet-bank field from their true edges, when snow began to bury
  /// the soil model's rock, which changes the snowmelt, and when water
  /// began to be routed by least transversal deviation and across flats
  /// towards their way out, when it drew rivers along smooth centrelines,
  /// when it joined tributaries at acute angles, narrowed steep reaches
  /// and flared estuaries, when bends began to migrate and only streams
  /// too narrow to migrate kept their drawn loops, and when centrelines
  /// were smoothed and resampled with coarser, quicker searches, and when
  /// bends began to be carved asymmetrically and steep, wide reaches to
  /// braid, and when ribbons gained their distance along the centreline,
  /// eddies and inner edges that never fold, and when the flat bank
  /// strips became banks with a margin, a face and a turf lip, and when
  /// steep streams began to step from pool to pool, and when rivers began
  /// to carry their catchment's colour on their swirl, and when streams
  /// narrower than 0.75 m stopped stepping.)
  #[test]
  fn the_mesa_river_build_is_bit_identical() {
    let options = vista_types::FractalTerrainOptions {
      seed: 12345,
      size: 512,
      horizontal_scale_metres: 30.0,
      vertical_scale: 1.0,
      base_height_metres: None,
      sea_level_metres: Some(0.0),
      noise: vista_types::NoiseOptions {
        kind: vista_types::NoiseKind::Ridged,
        octaves: 7,
        gain: 0.52,
        lacunarity: 2.05,
        warp: Some(0.15),
      },
      shape: Some(vista_types::TerrainShapeOptions {
        island: Some(0.0),
        ..Default::default()
      }),
      erosion: None,
      landform: vista_types::LandformKind::MesaDesert,
      edges: vista_types::TerrainEdges::Open,
    };
    let mut map = crate::terrain::generate_fractal_heightmap(&options).unwrap();
    let (_, surface) = crate::render::terrain_mesh::bake_terrain_shading(
      &map,
      &vista_types::BiomeOptions::default(),
      None,
    );
    let record = CarveRecord::new(map.heights.len());
    let network = build_river_network(
      &mut map,
      &RiverOptions::default(),
      RiverSources {
        surface: &surface,
        seed: 12345,
        painted: Vec::new(),
        record,
      },
    );
    assert!((network.inflows[0].discharge - 122.8).abs() < 0.1);

    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    fingerprint(&mut hash, bytemuck::cast_slice(&map.heights));

    for reach in &network.reaches {
      for p in &reach.points {
        let values = [
          p.x,
          p.y,
          p.level,
          p.bed,
          p.width,
          p.depth,
          p.discharge,
          p.slope,
          p.speed,
          p.curvature,
          p.celsius,
          p.rapids,
        ];
        fingerprint(&mut hash, bytemuck::cast_slice(&values));
        fingerprint(&mut hash, &[u8::from(p.falling)]);
      }
    }

    fingerprint(&mut hash, bytemuck::cast_slice(&network.vertices));
    fingerprint(&mut hash, bytemuck::cast_slice(&network.indices));
    fingerprint(&mut hash, bytemuck::cast_slice(&network.fall_vertices));
    fingerprint(&mut hash, bytemuck::cast_slice(&network.fall_indices));
    fingerprint(&mut hash, bytemuck::cast_slice(&network.bank_vertices));
    fingerprint(&mut hash, bytemuck::cast_slice(&network.bank_indices));
    fingerprint(&mut hash, &network.riparian);
    fingerprint(&mut hash, &network.wet.distance);
    fingerprint(&mut hash, &network.wet.still);
    let mask: Vec<u8> = network.mask.iter().map(|&wet| u8::from(wet)).collect();
    fingerprint(&mut hash, &mask);

    for (index, bed) in &network.bed {
      fingerprint(&mut hash, &index.to_le_bytes());
      fingerprint(&mut hash, bed);
    }

    for brook in &network.brooks {
      fingerprint(&mut hash, bytemuck::cast_slice(brook));
    }

    assert_eq!(hash, 0x3898_1ae9_c93e_baf4, "{hash:#x}");
  }
}
