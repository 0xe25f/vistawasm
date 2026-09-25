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
use crate::terrain::biomes::SurfaceSample;
use crate::terrain::channels::{
  channel_depth, condition_channels, height_at, kinoshita, kinoshita_table, raw_streams,
  CarveRecord, ChannelContext, ChannelPoint, Fall, FallStep, Oxbow, RawStream, Reach, GRAVITY,
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
  /// grid, half width for river ribbons, impact speed for falls, sprite
  /// size for mist.
  pub params: [f32; 3],
  /// Rivers: slope, curvature (-1 to 1), depth (metres), °C. Lakes:
  /// unused, unused, depth, °C. Falls: metres travelled down the sheet,
  /// sheet length, °C, height. Pools: bowl depth, fall height times
  /// discharge, unused, °C. Mist: fall height, discharge, °C, pool
  /// radius.
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
}

/// One bank strip vertex (36 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BankVertex {
  /// World position in terrain metres.
  pub position: [f32; 3],
  /// Unit direction away from the water (x, z).
  pub outward: [f32; 2],
  /// x: 0 at the water's edge to 1 at the strip's outer edge, y: strip
  /// width in metres, z: flow speed in metres per second, w: unused.
  pub params: [f32; 4],
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

  let anchored = |point: &ChannelPoint| {
    let (x, y) = (point.x.round() as i32, point.y.round() as i32);
    let end = |x: i32, y: i32| {
      x >= 0
        && y >= 0
        && x < map_width as i32
        && y < map_height as i32
        && ends[(y as u32 * map_width + x as u32) as usize]
    };
    (-1..=1).any(|dy| (-1..=1).any(|dx| end(x + dx, y + dy)))
      || hydrology.lake[cell_at(&hydrology, [point.x, point.y]) as usize] != NO_LAKE
  };

  let table = kinoshita_table();

  for (index, reach) in channels.reaches.iter().enumerate() {
    let phase = crate::maths::hash_u64(seed ^ index as u64) as f32 / u64::MAX as f32;
    let centre = sub_sample_centreline(
      &reach.points,
      metres,
      options.meanders,
      phase,
      &table,
      &anchored,
    );
    let drawn = add_ribbon(&mut network, &centre, metres, half, current, &anchored);
    add_bank_strips(&mut network, map, &drawn, metres, half);
    add_brooks(&mut network, &drawn, metres, half);
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
        params: [kind, across, *half_width],
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
/// has no finer detail to follow), then straight, uniform runs are
/// thinned out (see [`simplify_ribbon`]). Returns the points drawn.
fn add_ribbon(
  network: &mut RiverNetwork,
  points: &[ChannelPoint],
  metres: f32,
  half: [f32; 2],
  current: f32,
  anchored: &dyn Fn(&ChannelPoint) -> bool,
) -> Vec<ChannelPoint> {
  if points.len() < 2 {
    return points.to_vec();
  }

  let mut dense: Vec<ChannelPoint> = vec![points[0]];
  // Which dense points are the channel's own, not subdivisions.
  let mut original = vec![true];

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
      original.push(k == pieces);
    }
  }

  let dense = simplify_ribbon(&dense, &original, metres, anchored);
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
      // The flow always carries the direction, which the shader needs to
      // widen far ribbons across it, even with the current stopped.
      let speed = (p.speed * current).max(0.001);
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
  dense
}

/// Loops of the sub-sample centreline stay this far, in heightmap
/// samples, from the carved path, inside the carved trench.
pub const CORRIDOR_SAMPLES: f32 = 0.45;

/// The centreline a stream is drawn along. Streams narrower than a
/// heightmap sample on slopes under 1 % meander at their own wavelength
/// (about 11 w), which a grid 12 to 30 m apart cannot carve: a Kinoshita
/// curve of amplitude up to 2.5 w, kept within [`CORRIDOR_SAMPLES`] of
/// the carved path and pinned at its ends, at joins (`joined`), falls
/// and wider water. The
/// hydrology, carving and sounds keep the carved path.
fn sub_sample_centreline(
  points: &[ChannelPoint],
  metres: f32,
  strength: f32,
  phase: f32,
  table: &[f32; 64],
  joined: &dyn Fn(&ChannelPoint) -> bool,
) -> Vec<ChannelPoint> {
  let strength = strength.clamp(0.0, 1.0);
  // Loops shorter than a sixth of a sample would need many points for
  // little to see.
  let amplitude = |p: &ChannelPoint| {
    let loops = p.width < metres && 11.0 * p.width >= metres / 6.0 && !p.falling;
    let gentle = 1.0 - smoothstep((p.slope - 0.006) / 0.004);
    f32::from(u8::from(loops)) * gentle * (2.5 * p.width * strength).min(CORRIDOR_SAMPLES * metres)
  };

  if points.len() < 3 || strength <= 0.0 || points.iter().all(|p| amplitude(p) < 0.02 * metres) {
    return points.to_vec();
  }

  let mut out = vec![points[0]];
  let mut phase = phase;
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

  for pair in points.windows(2) {
    let (a, b) = (pair[0], pair[1]);
    let length = length2(b.x - a.x, b.y - a.y) * metres;
    let normal = [
      -(b.y - a.y) * metres / length.max(1e-4),
      (b.x - a.x) * metres / length.max(1e-4),
    ];
    let wavelength = 11.0 * a.width.min(b.width);
    let pieces = if amplitude(&a).max(amplitude(&b)) > 0.0 {
      (length / (wavelength / 8.0)).ceil().clamp(1.0, 256.0) as usize
    } else {
      1
    };

    for k in 1..=pieces {
      let t = k as f32 / pieces as f32;
      let lerp = |u: f32, v: f32| u + (v - u) * t;
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
      phase += step / (11.0 * p.width).max(0.01);

      while pin_index + 2 < pins.len() && pins[pin_index + 1] <= along {
        pin_index += 1;
      }

      // Pinned over half a wavelength around each pin, so joins meet.
      let reach = 5.5 * p.width;
      let pin = smoothstep((along - pins[pin_index]) / reach)
        * smoothstep((pins[pin_index + 1] - along) / reach);
      let offset =
        amplitude(&p).min(amplitude(&a)).min(amplitude(&b)) * pin * kinoshita(table, phase);
      p.x += normal[0] * offset / metres;
      p.y += normal[1] * offset / metres;
      out.push(p);
    }
  }

  out
}

/// Bank strips on either side of a stream narrower than a heightmap
/// sample, from 0.5 w out to 0.5 w + b, where b = max(0.5 m, 0.4 w), 2 cm
/// over the ground as the terrain mesh draws it. The heightmap has only a
/// trench one sample wide there, so the strips give it crisp banks. Where
/// that trench lies below the water surface, the strip rises to the
/// surface, or it would slide out from under the water at low angles.
fn add_bank_strips(
  network: &mut RiverNetwork,
  map: &HeightMap,
  points: &[ChannelPoint],
  metres: f32,
  half: [f32; 2],
) {
  let n = points.len();

  for side in [-1.0f32, 1.0] {
    let mut row = 0;

    for i in 0..n {
      let p = &points[i];
      let (prev, next) = (&points[i.saturating_sub(1)], &points[(i + 1).min(n - 1)]);
      let tangent = [next.x - prev.x, next.y - prev.y];
      let length = length2(tangent[0], tangent[1]);
      let narrow = p.width < metres && !p.falling && length > 1e-5;

      if !narrow {
        row = 0;
        continue;
      }

      let outward = [-tangent[1] / length * side, tangent[0] / length * side];
      let strip = (0.4 * p.width).max(0.5);
      let first = network.bank_vertices.len() as u32;

      for (t, offset) in [(0.0, 0.5 * p.width), (1.0, 0.5 * p.width + strip)] {
        let x = p.x + outward[0] * offset / metres;
        let y = p.y + outward[1] * offset / metres;
        let world = to_world([x, y], metres, half);
        network.bank_vertices.push(BankVertex {
          position: [
            world[0],
            (mesh_height_at(map, x, y) + 0.02).max(p.level),
            world[1],
          ],
          outward,
          params: [t, strip, p.speed, 0.0],
        });
      }

      if row > 0 {
        let (a, b) = (first - 2, first - 1);
        network
          .bank_indices
          .extend_from_slice(&[a, first, b, b, first, first + 1]);
      }

      row += 1;
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
/// where the centreline strays less than 0.05 w and 0.1 m from the chord
/// between the points kept either side, and width, depth, speed, slope and
/// level differ from their interpolation along the chord by less than
/// 2 %. The ends, and of the channel's own points (`original`; the rest
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

      off_chord < 0.05 * p.width
        && off_chord < 0.1
        && close(p.width, lerp(pa.width, pc.width), p.width)
        && close(p.depth, lerp(pa.depth, pc.depth), p.depth)
        && close(p.speed, lerp(pa.speed, pc.speed), p.speed)
        && close(p.slope, lerp(pa.slope, pc.slope), p.slope.max(0.005))
        && close(p.level, lerp(pa.level, pc.level), p.depth)
        && (p.curvature - lerp(pa.curvature, pc.curvature)).abs() <= 0.02
        && (p.celsius - lerp(pa.celsius, pc.celsius)).abs() <= 0.1
    })
  };
  let mut kept = vec![points[0]];
  let mut a = 0;

  while a < n - 1 {
    let mut j = a + 1;

    while j + 1 < n && !keep[j] && j + 1 - a <= MAX_RUN && fits(a, j + 1) {
      j += 1;
    }

    kept.push(points[j]);
    a = j;
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
    let ground = height_at(map, x, y).max(mesh_height_at(map, x, y));
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
    params: [WATER_KIND_POOL, 0.0, 0.0],
    extra: pool,
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
        params: [WATER_KIND_FALL, across, impact],
        extra: [travelled, total, fall.celsius, height],
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
      })
      .collect()
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

  #[test]
  fn narrow_stream_loops_pass_through_joins() {
    let table = kinoshita_table();
    let carved = flat_brook(2.0);
    let join = carved[80];
    let centre = sub_sample_centreline(&carved, 30.0, 1.0, 0.3, &table, &|point| *point == join);
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
    let centre = sub_sample_centreline(&carved, 30.0, 1.0, 0.3, &table, &|_| false);
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
      sub_sample_centreline(&wide, 30.0, 1.0, 0.3, &table, &|_| false),
      wide
    );
    assert_eq!(
      sub_sample_centreline(&carved, 30.0, 0.0, 0.3, &table, &|_| false),
      carved
    );
  }

  #[test]
  fn only_streams_narrower_than_a_sample_get_bank_strips() {
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
    let mut network = RiverNetwork::default();
    add_bank_strips(&mut network, &map, &low, 30.0, [1905.0, 945.0]);
    assert!(!network.bank_vertices.is_empty());
    assert!(network
      .bank_vertices
      .iter()
      .all(|vertex| (vertex.position[1] - 5.02).abs() < 1e-4));
    let strip = network.bank_vertices[1].params[1];
    assert!((strip - 0.8).abs() < 1e-4, "strip {strip} m");

    // Never below the water surface of a stream over a coarse trench.
    let mut raised = RiverNetwork::default();
    let high: Vec<_> = flat_brook(2.0)
      .into_iter()
      .map(|mut point| {
        point.level = 5.5;
        point
      })
      .collect();
    add_bank_strips(&mut raised, &map, &high, 30.0, [1905.0, 945.0]);
    assert!(raised
      .bank_vertices
      .iter()
      .all(|vertex| (vertex.position[1] - 5.5).abs() < 1e-4));

    let mut wide = RiverNetwork::default();
    add_bank_strips(&mut wide, &map, &flat_brook(40.0), 30.0, [1905.0, 945.0]);
    assert!(wide.bank_vertices.is_empty());
  }

  #[test]
  fn reeds_line_the_true_banks_of_slow_brooks_on_coarse_maps() {
    use crate::render::flora::GRASS_STYLE_REED;
    use crate::render::grass::build_grass_instances_by_water;
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
    let instances = build_grass_instances_by_water(
      &map,
      Some(&warm),
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
      .filter(|vertex| vertex.params[0] == WATER_KIND_POOL)
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
      let ground = height_at(&map, x, y).max(mesh_height_at(&map, x, y));
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
      .any(|vertex| vertex.params[0] == WATER_KIND_POOL));
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
