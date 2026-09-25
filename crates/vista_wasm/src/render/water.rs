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

use crate::maths::length2;
use crate::terrain::biomes::SurfaceSample;
use crate::terrain::channels::{
  channel_depth, condition_channels, height_at, raw_streams, CarveRecord, ChannelContext,
  ChannelPoint, Fall, Oxbow, RawStream, Reach, GRAVITY,
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

/// One water vertex (48 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WaterVertex {
  /// Ocean: local offset from the camera-snapped grid origin (y unused).
  /// Rivers and lakes: world position in terrain metres.
  pub position: [f32; 3],
  /// Surface current in metres per second (x, z).
  pub flow: [f32; 2],
  /// x: kind (`WATER_KIND_*`), y: across-channel coordinate (-1 to 1) for
  /// rivers and falls, z: local vertex spacing in metres for the ocean
  /// grid, impact speed for falls, sprite size for mist.
  pub params: [f32; 3],
  /// Rivers: slope, curvature (-1 to 1), depth (metres), °C. Lakes:
  /// unused, unused, depth, °C. Falls: metres travelled down the sheet,
  /// sheet length, °C, height. Pools: radius, fall height, °C,
  /// discharge. Mist: fall height, discharge, °C, pool radius.
  pub extra: [f32; 4],
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
    params: [WATER_KIND_OCEAN, 0.0, 0.0],
    extra: [0.0; 4],
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
        params: [WATER_KIND_OCEAN, 0.0, spacing_x.max(spacing_z)],
        extra: [0.0; 4],
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
    let map_width = map.metadata.width;
    let map_height = map.metadata.height;

    if map_width < 2 || map_height < 2 || !water.iter().any(|w| *w) {
      return Self::default();
    }

    let stride = (map_width.max(map_height).saturating_sub(1) / (WET_BANK_MAX - 1)).max(1);
    let width = (map_width - 1) / stride + 1;
    let height = (map_height - 1) / stride + 1;
    let step = map.metadata.metres_per_sample.max(0.001) * stride as f32;
    let field = |seeds: &[bool]| {
      let mut distance: Vec<f32> = (0..width * height)
        .map(|texel| {
          let x = (texel % width) * stride;
          let y = (texel / width) * stride;
          if seeds[(y * map_width + x) as usize] {
            0.0
          } else {
            f32::MAX
          }
        })
        .collect();
      crate::terrain::biomes::chamfer_distance(
        width as usize,
        height as usize,
        step,
        &mut distance,
      );
      distance
        .iter()
        .map(|d| {
          let edge = if *d > 0.0 {
            (d - step * 0.5).max(0.0)
          } else {
            0.0
          };
          (edge / WET_BANK_RANGE_METRES * 255.0).round().min(255.0) as u8
        })
        .collect()
    };

    Self {
      width,
      height,
      stride,
      distance: field(water),
      still: field(still),
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

  if map_width < 8
    || map_height < 8
    || (!options.enabled && painted.is_empty() && record.is_empty())
  {
    network.carved = record.into_original();
    return network;
  }

  let hydrology = build_hydrology(map, surface, options, seed);
  let mut painted_cells = vec![false; hydrology.ground.len()];
  let painted: Vec<RawStream> = painted
    .iter()
    .filter_map(|river| painted_stream(&hydrology, map, river, &mut painted_cells))
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

  for reach in &channels.reaches {
    add_ribbon(&mut network, &reach.points, metres, half, current);
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

  for reach in &channels.reaches {
    for point in reach.points.iter().filter(|point| point.speed < REED_SPEED) {
      mark_sample(&mut still, map, [point.x, point.y]);
    }
  }

  for (slot, value) in network.mask.iter_mut().zip(&channels.mask) {
    *slot |= *value;
  }

  network.wet = WetBanks::build(map, &network.mask, &still);

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
  crate::terrain::heightmap::update_stats(&map.heights, &map.no_data, &mut map.metadata);
  network
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
/// channel, a lake, the sea or the map edge, so it joins the network.
fn painted_stream(
  hydrology: &Hydrology,
  map: &HeightMap,
  river: &PaintedRiver,
  painted_cells: &mut [bool],
) -> Option<RawStream> {
  let mut points = river.points.clone();
  let mut levels: Vec<f32> = points.iter().map(|p| height_at(map, p[0], p[1])).collect();
  let mut discharge: Vec<f32> = points
    .iter()
    .map(|p| hydrology.discharge[cell_at(hydrology, *p) as usize])
    .collect();

  for point in &points {
    painted_cells[cell_at(hydrology, *point) as usize] = true;
  }

  let mut cell = cell_at(hydrology, *points.last()?);
  let mouth = loop {
    let r = hydrology.receiver[cell as usize];

    if r == NO_RECEIVER {
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
    discharge.push(hydrology.discharge[r as usize].max(*discharge.last()?));

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
    min_width: river.width,
    mouth,
  })
}

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

/// Ground height at sample coordinates as the terrain mesh draws it near
/// the camera: two triangles per sample square, split from its top-right
/// to its bottom-left corner. Over a sharply carved sample this differs
/// from the bilinear height by metres.
fn mesh_height_at(map: &HeightMap, x: f32, y: f32) -> f32 {
  let width = map.metadata.width;
  let height = map.metadata.height;

  if width < 2 || height < 2 {
    return height_at(map, x, y);
  }

  let fx = x.clamp(0.0, (width - 1) as f32);
  let fy = y.clamp(0.0, (height - 1) as f32);
  let x0 = (fx as u32).min(width - 2);
  let y0 = (fy as u32).min(height - 2);
  let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
  let at = |x: u32, y: u32| map.heights[(y * width + x) as usize];
  let (top_left, top_right) = (at(x0, y0), at(x0 + 1, y0));
  let (bottom_left, bottom_right) = (at(x0, y0 + 1), at(x0 + 1, y0 + 1));

  if tx + ty <= 1.0 {
    top_left + (top_right - top_left) * tx + (bottom_left - top_left) * ty
  } else {
    bottom_right
      + (bottom_left - bottom_right) * (1.0 - tx)
      + (top_right - bottom_right) * (1.0 - ty)
  }
}

fn to_world(point: [f32; 2], metres: f32, half: [f32; 2]) -> [f32; 2] {
  [point[0] * metres - half[0], point[1] * metres - half[1]]
}

/// One row across a strip: centre, unit side vector, half width, level,
/// flow and extra vertex data.
type StripRow = ([f32; 2], [f32; 2], f32, f32, [f32; 2], [f32; 4]);

/// Push one ribbon strip. `rows` are (centre, side, half width, level,
/// flow, extra) per row; `skip` leaves out the quads after a row.
fn push_strip(
  vertices: &mut Vec<WaterVertex>,
  indices: &mut Vec<u32>,
  kind: f32,
  rows: &[StripRow],
  skip: &[bool],
) {
  let first = vertices.len() as u32;

  for (centre, side, half_width, level, flow, extra) in rows {
    for across in [-1.0f32, 1.0] {
      vertices.push(WaterVertex {
        position: [
          centre[0] + side[0] * half_width * across,
          *level,
          centre[1] + side[1] * half_width * across,
        ],
        flow: *flow,
        params: [kind, across, 0.0],
        extra: *extra,
      });
    }
  }

  for i in 0..rows.len().saturating_sub(1) as u32 {
    if skip.get(i as usize).copied().unwrap_or(false) {
      continue;
    }

    let a = first + i * 2;
    indices.extend_from_slice(&[a, a + 1, a + 2, a + 2, a + 1, a + 3]);
  }
}

/// A river ribbon, 1.3 w wide, so its edge lies on the bank, where the
/// shader fades it out by depth. Steep reaches are subdivided so no
/// segment is longer than half the width (or half a sample: the ground
/// has no finer detail to follow).
fn add_ribbon(
  network: &mut RiverNetwork,
  points: &[ChannelPoint],
  metres: f32,
  half: [f32; 2],
  current: f32,
) {
  if points.len() < 2 {
    return;
  }

  let mut dense: Vec<ChannelPoint> = vec![points[0]];

  for pair in points.windows(2) {
    let (a, b) = (pair[0], pair[1]);
    let length = length2(b.x - a.x, b.y - a.y) * metres;
    let limit = if a.slope.max(b.slope) > 0.02 {
      (0.5 * a.width.min(b.width)).max(0.5 * metres)
    } else {
      metres
    };
    let pieces = (length / limit).ceil().clamp(1.0, 64.0) as usize;

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
    }
  }

  let n = dense.len();
  let world: Vec<[f32; 2]> = dense
    .iter()
    .map(|p| to_world([p.x, p.y], metres, half))
    .collect();
  let rows: Vec<_> = (0..n)
    .map(|i| {
      let p = &dense[i];
      let prev = world[i.saturating_sub(1)];
      let next = world[(i + 1).min(n - 1)];
      let tangent = [next[0] - prev[0], next[1] - prev[1]];
      let length = length2(tangent[0], tangent[1]).max(1e-4);
      let tangent = [tangent[0] / length, tangent[1] / length];
      let half_width = 0.65 * p.width;
      let speed = p.speed * current;
      (
        world[i],
        [-tangent[1], tangent[0]],
        half_width,
        p.level,
        [tangent[0] * speed, tangent[1] * speed],
        [p.slope, p.curvature, p.depth, p.celsius],
      )
    })
    .collect();
  let skip: Vec<bool> = (0..n)
    .map(|i| dense[i].falling && dense[(i + 1).min(n - 1)].falling)
    .collect();
  push_strip(
    &mut network.vertices,
    &mut network.indices,
    WATER_KIND_RIVER,
    &rows,
    &skip,
  );
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
      (
        world[i],
        [-tangent[1] / length, tangent[0] / length],
        half_width,
        oxbow.surface,
        [0.0, 0.0],
        [0.0, 0.0, 0.0, oxbow.celsius],
      )
    })
    .collect();
  push_strip(
    &mut network.vertices,
    &mut network.indices,
    WATER_KIND_LAKE,
    &rows,
    &[],
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
          params: [WATER_KIND_LAKE, 0.0, 0.0],
          extra,
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

/// Rows down a waterfall sheet.
const FALL_ROWS: usize = 12;

/// A waterfall: the churned plunge pool (with the other water surfaces),
/// and in a buffer of their own, the falling sheet and mist sprites at its
/// foot.
fn add_fall(
  network: &mut RiverNetwork,
  map: &HeightMap,
  fall: &Fall,
  metres: f32,
  half: [f32; 2],
  seed: u64,
) {
  let height = fall.height();
  let speed = fall.speed.max(0.5);
  let fall_time = (2.0 * height / GRAVITY).sqrt();
  let run = length2(fall.foot[0] - fall.lip[0], fall.foot[1] - fall.lip[1]) * metres;
  let reach = (speed * fall_time).max(run);
  let columns = ((fall.width / 4.0).ceil() as usize).max(3);
  let direction = fall.direction;
  let side = [-direction[1], direction[0]];
  let impact = (2.0 * GRAVITY * height).sqrt();
  let foot = to_world(fall.foot, metres, half);

  // The churned plunge pool. Its water stands only as high as where it
  // spills: the lowest ground just outside its rim, or the water in its
  // outlet channel, and at most the foot. Below that it is flat; above
  // it, and all over where the bowl holds nothing, as on a slope, it is a
  // thin film over the ground, so it never stands out as a shelf. The
  // shader fades it towards its rim, so where the pool is smaller than a
  // heightmap sample it does not end in a hard edge.
  let ring = fall.pool_radius / metres + 0.5;
  let mut level = fall.foot_level;

  for k in 0..POOL_SEGMENTS {
    let angle = k as f32 / POOL_SEGMENTS as f32 * std::f32::consts::TAU;
    let (cos, sin) = (angle.cos(), angle.sin());
    let ground = height_at(map, fall.foot[0] + cos * ring, fall.foot[1] + sin * ring);
    let outlet = cos * direction[0] + sin * direction[1] > 0.77;
    level = level.min(if outlet {
      ground + channel_depth(fall.discharge)
    } else {
      ground
    });
  }

  let surface = |x: f32, y: f32| {
    let ground = height_at(map, x, y).max(mesh_height_at(map, x, y));
    fall.foot_level.min(level.max(ground + POOL_FILM_METRES))
  };
  let held = (level - (fall.foot_level - fall.pool_depth)).max(0.0);
  let centre = network.vertices.len() as u32;
  let pool = [held, height, fall.celsius, fall.discharge];
  network.vertices.push(WaterVertex {
    position: [foot[0], surface(fall.foot[0], fall.foot[1]) + 0.02, foot[1]],
    flow: [0.0, 0.0],
    params: [WATER_KIND_POOL, 0.0, 0.0],
    extra: pool,
  });

  let rings = ((3.0 * fall.pool_radius / metres).ceil() as usize).clamp(2, POOL_RINGS);

  for ring in 1..=rings {
    let across = ring as f32 / rings as f32;
    let radius = fall.pool_radius * across;

    for k in 0..POOL_SEGMENTS {
      let angle = k as f32 / POOL_SEGMENTS as f32 * std::f32::consts::TAU;
      let (cos, sin) = (angle.cos(), angle.sin());
      let level = surface(
        fall.foot[0] + cos * radius / metres,
        fall.foot[1] + sin * radius / metres,
      );
      network.vertices.push(WaterVertex {
        position: [foot[0] + cos * radius, level + 0.02, foot[1] + sin * radius],
        flow: [cos, sin],
        params: [WATER_KIND_POOL, across, 0.0],
        extra: pool,
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

  let vertices = &mut network.fall_vertices;
  let indices = &mut network.fall_indices;

  // The sheet follows the path of water leaving the lip, x = v t and
  // y = -g t^2 / 2, but never cuts into the rock: where the face is less
  // than vertical it is pushed out to lie just over it.
  let mut rows = Vec::with_capacity(FALL_ROWS + 1);
  let mut travelled = 0.0;

  for k in 0..=FALL_ROWS {
    let along = reach * k as f32 / FALL_ROWS as f32;
    let t = along / speed;
    let projectile = if t <= fall_time {
      fall.lip_level - 0.5 * GRAVITY * t * t
    } else {
      fall.foot_level
    };
    let sx = fall.lip[0] + direction[0] * along / metres;
    let sy = fall.lip[1] + direction[1] * along / metres;
    let rock = height_at(map, sx, sy);
    let y = if k == FALL_ROWS {
      fall.foot_level
    } else {
      projectile.max(rock + 0.3).max(fall.foot_level)
    };

    if let Some((_, _, py)) = rows.last() {
      let py: f32 = *py;
      travelled += length2(reach / FALL_ROWS as f32, py - y);
    }

    rows.push((sx, sy, y));
  }

  let total = travelled.max(0.01);
  let mut travelled = 0.0;
  let first = vertices.len() as u32;

  for (k, (sx, sy, y)) in rows.iter().enumerate() {
    if k > 0 {
      travelled += length2(reach / FALL_ROWS as f32, rows[k - 1].2 - y);
    }

    for column in 0..=columns {
      let across = column as f32 / columns as f32 * 2.0 - 1.0;
      let offset = across * fall.width * 0.5 / metres;
      let world = to_world([sx + side[0] * offset, sy + side[1] * offset], metres, half);
      vertices.push(WaterVertex {
        position: [world[0], *y, world[1]],
        flow: [direction[0] * speed, direction[1] * speed],
        params: [WATER_KIND_FALL, across, impact],
        extra: [travelled, total, fall.celsius, height],
      });
    }
  }

  let stride = columns as u32 + 1;

  for k in 0..FALL_ROWS as u32 {
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
        params: [WATER_KIND_SPRAY, unit(32), size * (0.6 + 0.4 * unit(48))],
        extra: [height, fall.discharge, fall.celsius, fall.pool_radius],
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

  #[test]
  fn lakes_freeze_below_zero_margins_first_and_fully_at_minus_two() {
    assert_eq!(freeze_fraction(-3.0, LAKE_FREEZE_CELSIUS), 1.0);
    assert_eq!(freeze_fraction(1.0, LAKE_FREEZE_CELSIUS), 0.0);
    let partial = freeze_fraction(-1.0, LAKE_FREEZE_CELSIUS);
    assert!(partial > 0.0 && partial < 1.0);
    assert_eq!(freeze_fraction(-6.0, RIVER_FREEZE_CELSIUS), 0.5);
    assert_eq!(freeze_fraction(-10.0, FALL_FREEZE_CELSIUS), 1.0);
  }

  #[test]
  fn mesh_heights_follow_the_terrain_triangles() {
    let metadata = TerrainMetadata {
      metres_per_sample: 10.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(3, 3, 0.0, metadata);
    // Only the bottom-right corner of the first square is raised, so the
    // top-left triangle stays flat and the bilinear height does not.
    map.heights[4] = 8.0;

    assert_eq!(mesh_height_at(&map, 0.4, 0.4), 0.0);
    assert!(height_at(&map, 0.4, 0.4) > 1.0);
    assert!((mesh_height_at(&map, 0.9, 0.9) - 6.4).abs() < 1e-4);
    assert_eq!(mesh_height_at(&map, 1.0, 1.0), 8.0);
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
    let options = RiverOptions {
      min_catchment_km2: 0.005,
      ..RiverOptions::default()
    };
    let sources = plain(map.heights.len());
    let network = build_river_network(&mut map, &options, sources);
    let half = (size as f32 - 1.0) * 2.0 * 0.5;
    let pools: Vec<&WaterVertex> = network
      .vertices
      .iter()
      .filter(|vertex| vertex.params[0] == WATER_KIND_POOL)
      .collect();

    assert!(!network.falls.is_empty());
    assert!(!pools.is_empty());

    // No part of a pool stands clear of the ground: at most a film over
    // it, or, towards the outlet, the depth of the stream leaving it.
    for vertex in pools {
      let x = (vertex.position[0] + half) / 2.0;
      let y = (vertex.position[2] + half) / 2.0;
      let ground = height_at(&map, x, y).max(mesh_height_at(&map, x, y));
      let above = vertex.position[1] - ground;
      let allowed = POOL_FILM_METRES + channel_depth(vertex.extra[3]) + 0.03;

      assert!(
        above <= allowed,
        "pool {above} m above the ground at {x}, {y}"
      );
    }
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
      .filter(|vertex| vertex.params[0] == WATER_KIND_RIVER)
      .map(|vertex| vertex.position[0])
      .collect();
    let mean_x = river_x.iter().sum::<f32>() / river_x.len() as f32;
    assert!(mean_x.abs() < 400.0, "mean river x {mean_x}");

    // Every river vertex flows north (towards lower y / negative z).
    assert!(network
      .vertices
      .iter()
      .filter(|vertex| vertex.params[0] == WATER_KIND_RIVER)
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

    assert!(network.vertices.iter().any(
      |vertex| vertex.params[0] == WATER_KIND_LAKE && (vertex.position[1] - 100.0).abs() < 0.5
    ));
  }
}
