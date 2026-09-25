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
  condition_channels, raw_streams, CarveRecord, ChannelContext, ChannelPoint, Fall, Oxbow,
  RawStream, Reach, GRAVITY,
};
use crate::terrain::heightmap::HeightMap;
use crate::terrain::hydrology::{build_hydrology, Hydrology};

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
  /// Waterfall sheets, spray and plunge pools, drawn after the rest.
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
  pub painted: Vec<RawStream>,
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
  let mut streams = if options.enabled {
    raw_streams(&hydrology)
  } else {
    Vec::new()
  };

  // Painted rivers win over the drainage: they are cut first, at their
  // painted width, and natural streams join them.
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

  for (slot, value) in network.mask.iter_mut().zip(&channels.mask) {
    *slot |= *value;
  }

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

/// Restore every sample changed by [`build_river_network`].
pub fn restore_carving(map: &mut HeightMap, carved: &[(usize, f32)]) {
  for (index, height) in carved {
    if let Some(slot) = map.heights.get_mut(*index) {
      *slot = *height;
    }
  }

  crate::terrain::heightmap::update_stats(&map.heights, &map.no_data, &mut map.metadata);
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

/// A river ribbon: at least 1.3 w wide, and wide enough to reach past the
/// carved banks, so the water's visible edge is always where the shader
/// fades it out against the bank. Steep reaches are subdivided so no
/// segment is longer than half the width (or a quarter of a sample).
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
      (0.5 * a.width.min(b.width)).max(0.25 * metres)
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
      let carved = (0.5 * p.width).max(0.6 * metres) + metres;
      let half_width = (0.65 * p.width).max(carved);
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
  let half_width = (0.65 * oxbow.width).max((0.5 * oxbow.width).max(0.6 * metres) + metres);
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

/// A waterfall: the falling sheet, mist sprites at its foot, and the
/// churned plunge pool.
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
    let rock = crate::terrain::channels::height_at(map, sx, sy);
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

  let foot = to_world(fall.foot, metres, half);

  // The churned plunge pool: a disc at the foot, a little wider than the
  // pool, faded out by depth where it meets the bank.
  let centre = vertices.len() as u32;
  let pool = [fall.pool_radius, height, fall.celsius, fall.discharge];
  let radius = fall.pool_radius * 1.15 + fall.width * 0.5;
  vertices.push(WaterVertex {
    position: [foot[0], fall.foot_level + 0.02, foot[1]],
    flow: [0.0, 0.0],
    params: [WATER_KIND_POOL, 0.0, 0.0],
    extra: pool,
  });

  for k in 0..24 {
    let angle = k as f32 / 24.0 * std::f32::consts::TAU;
    vertices.push(WaterVertex {
      position: [
        foot[0] + angle.cos() * radius,
        fall.foot_level + 0.02,
        foot[1] + angle.sin() * radius,
      ],
      flow: [angle.cos(), angle.sin()],
      params: [WATER_KIND_POOL, radius / fall.pool_radius.max(0.1), 0.0],
      extra: pool,
    });
  }

  for k in 0..24u32 {
    indices.extend_from_slice(&[centre, centre + 1 + (k + 1) % 24, centre + 1 + k]);
  }

  // Mist: 16 to 64 camera-facing sprites, more for bigger falls.
  let sprites = (16.0 + (fall.discharge * height).sqrt() * 4.0).clamp(16.0, 64.0) as u32;
  // Bigger falls throw up bigger clouds; a trickle only a light mist.
  let size = (0.25 * height + fall.width * 0.5)
    .min(4.0 + 30.0 * fall.discharge.sqrt())
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
