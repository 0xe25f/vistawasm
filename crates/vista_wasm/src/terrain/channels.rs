//! Channel conditioning: the final river stage.
//!
//! Streams come from [`crate::terrain::hydrology`] and follow the valleys
//! that erosion carved. This stage only shapes their beds and banks:
//!
//! - hydraulic geometry: width and depth grow with discharge;
//! - a water level and bed that never rise downstream, smoothed except
//!   across waterfall steps;
//! - a narrow V in steep ground and a flat-bottomed channel with a
//!   floodplain on gentle ground;
//! - meanders on flat lowland reaches, with the occasional oxbow lake;
//! - deltas where large rivers meet the sea on flat ground;
//! - waterfalls where the bed drops over a step, with a plunge pool.
//!
//! Every changed height is recorded, so the terrain can be restored
//! exactly when river options or the water mask change.

use vista_types::RiverOptions;

use std::f32::consts::FRAC_PI_2;

use crate::maths::{hash_u64, length2, smoothstep, value_noise};
use crate::terrain::biomes::SurfaceSample;
use crate::terrain::centreline::{continuous_centreline, floor_widths, simplify};
use crate::terrain::heightmap::HeightMap;
use crate::terrain::hydrology::{Hydrology, Mouth, NO_LAKE};
use crate::terrain::meander::{braiding_threshold, migrate, representable, CutLoop, Migration};

/// Manning roughness of a natural lowland channel.
const MANNING_N: f32 = 0.035;

/// Gravity, in metres per second squared.
pub const GRAVITY: f32 = 9.81;

/// Unit stream power, 1000 x g x Q x S / w, in W/m².
pub fn stream_power(point: &ChannelPoint) -> f32 {
  1000.0 * GRAVITY * point.discharge * point.slope / point.width.max(0.1)
}

/// How far a reach has cut down to bedrock: 0 where its stream power is
/// 300 W/m² or less, its slope 2 % or less or its discharge 1 m³/s or
/// less, rising to 1 at 600 W/m², 3 % and 4 m³/s. Its banks become rock
/// walls, steepening towards vertical, and its stones grow. A rivulet
/// is steep enough for that power but runs over soil and boulders, not
/// in a bedrock trench. Outcrops and boulders can read the same rule.
pub fn rock_banks(point: &ChannelPoint) -> f32 {
  smoothstep((stream_power(point) - 300.0) / 300.0)
    * smoothstep((point.slope - 0.02) / 0.01)
    * smoothstep((point.discharge - 1.0) / 3.0)
}

/// Valley slope above which channels are cut as a V.
const V_SLOPE: f32 = 0.06;

/// Valley slope below which channels are flat-bottomed with a floodplain.
const FLAT_SLOPE: f32 = 0.02;

/// Deltas form on channels wider than this, in metres.
const DELTA_WIDTH: f32 = 8.0;

/// Deltas form where the last reach is flatter than this.
const DELTA_SLOPE: f32 = 0.005;

/// Rivers at least this wide, in metres, on valley slopes under
/// [`VALLEY_FLOOR_SLOPE`], lower a valley floor beside their banks.
pub const VALLEY_FLOOR_WIDTH: f32 = 20.0;

/// See [`VALLEY_FLOOR_WIDTH`].
const VALLEY_FLOOR_SLOPE: f32 = 0.01;

/// A step lower than this is a rapid, not a fall.
const MIN_FALL_METRES: f32 = 3.0;

/// Channel width in metres for a discharge, before `widthScale`.
pub fn channel_width(discharge: f32, width_scale: f32) -> f32 {
  (2.7 * discharge.max(0.0).sqrt() * width_scale).clamp(0.6, 400.0)
}

/// The discharge a channel `width` metres wide carries, by
/// [`channel_width`] turned round.
pub fn width_discharge(width: f32, width_scale: f32) -> f32 {
  (width.max(0.0) / (2.7 * width_scale)).powi(2)
}

/// Channel depth in metres for a discharge.
pub fn channel_depth(discharge: f32) -> f32 {
  (0.35 * discharge.max(0.0).powf(0.4)).clamp(0.6, 400.0)
}

/// Mean flow speed from Manning's equation, with the hydraulic radius
/// taken as the depth, clamped to 0.2 to 6 m/s. Steep channels lose
/// their energy over boulders, steps and pools, so their roughness
/// grows with slope (Jarrett's relation for mountain streams): a 4 %
/// river 2 m deep runs at about 3 m/s, not at the ceiling.
pub fn manning_speed(depth: f32, slope: f32) -> f32 {
  let slope = slope.max(1.0e-5);
  let roughness = MANNING_N.max(0.39 * slope.powf(0.38) * depth.max(0.1).powf(-0.16));
  (depth.powf(2.0 / 3.0) * slope.sqrt() / roughness).clamp(0.2, 6.0)
}

/// Original heights of every sample the river stages change, so they can
/// be put back exactly.
#[derive(Clone, Debug, Default)]
pub struct CarveRecord {
  seen: Vec<bool>,
  original: Vec<(usize, f32)>,
}

impl CarveRecord {
  /// A record for a heightmap with `samples` samples.
  pub fn new(samples: usize) -> Self {
    Self {
      seen: vec![false; samples],
      original: Vec::new(),
    }
  }

  /// Set a sample's height, recording its original height the first time.
  pub fn set(&mut self, map: &mut HeightMap, index: usize, height: f32) {
    if !self.seen[index] {
      self.seen[index] = true;
      self.original.push((index, map.heights[index]));
    }

    map.heights[index] = height;
  }

  /// Lower a sample to `target` if it is higher.
  pub fn lower(&mut self, map: &mut HeightMap, index: usize, target: f32) {
    if target < map.heights[index] {
      self.set(map, index, target);
    }
  }

  /// Whether nothing has been changed.
  pub fn is_empty(&self) -> bool {
    self.original.is_empty()
  }

  /// The original heights, in the order they were first changed.
  pub fn into_original(self) -> Vec<(usize, f32)> {
    self.original
  }
}

/// One point along a channel centreline.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChannelPoint {
  /// Heightmap sample coordinates (fractional).
  pub x: f32,
  /// See `x`.
  pub y: f32,
  /// Water surface height in metres.
  pub level: f32,
  /// Bed height in metres.
  pub bed: f32,
  /// Channel width in metres.
  pub width: f32,
  /// Channel depth in metres.
  pub depth: f32,
  /// Mean discharge in cubic metres per second.
  pub discharge: f32,
  /// Valley slope along the channel (rise over run).
  pub slope: f32,
  /// Mean flow speed in metres per second.
  pub speed: f32,
  /// Signed curvature times width, -1 to 1; positive where the channel
  /// turns left (counter-clockwise seen from above).
  pub curvature: f32,
  /// Mean annual temperature in °C.
  pub celsius: f32,
  /// Extra whitewater from small steps, 0 to 1.
  pub rapids: f32,
  /// Whether the point is on a waterfall (from its lip to its foot).
  pub falling: bool,
  /// Strahler order of the stream here (0 where unknown).
  pub order: u8,
}

/// What a reach is drawn, carved and heard as.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReachKind {
  /// A channel: drawn, carved and heard.
  #[default]
  Main,
  /// A braided stretch's belt: carved flat and heard, but drawn only
  /// through its threads.
  Belt,
  /// One thread of a braided belt: drawn and carved, but not heard.
  Thread,
}

/// A channel centreline from upstream to downstream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reach {
  /// Points from upstream to downstream.
  pub points: Vec<ChannelPoint>,
  /// What it is drawn, carved and heard as.
  pub kind: ReachKind,
}

/// One step of a cascade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FallStep {
  /// Lip, in heightmap sample coordinates.
  pub lip: [f32; 2],
  /// Water level at the lip, in metres.
  pub lip_level: f32,
  /// Foot, in heightmap sample coordinates.
  pub foot: [f32; 2],
  /// Water level at the foot, in metres.
  pub foot_level: f32,
  /// Radius of the churned water at its foot, in metres.
  pub pool_radius: f32,
}

/// A waterfall where a channel drops over a step, or a cascade of falls
/// close together, from the first lip to the last foot.
#[derive(Clone, Debug, PartialEq)]
pub struct Fall {
  /// Lip, in heightmap sample coordinates.
  pub lip: [f32; 2],
  /// Water level at the lip, in metres.
  pub lip_level: f32,
  /// Foot, in heightmap sample coordinates.
  pub foot: [f32; 2],
  /// Water level at the foot, in metres.
  pub foot_level: f32,
  /// Unit direction the water leaves the lip in (sample axes).
  pub direction: [f32; 2],
  /// Width of the falling water, in metres.
  pub width: f32,
  /// Mean discharge in cubic metres per second.
  pub discharge: f32,
  /// Speed the water leaves the lip at, in metres per second.
  pub speed: f32,
  /// Radius of the plunge pool at the foot, in metres.
  pub pool_radius: f32,
  /// Depth of the plunge pool, in metres.
  pub pool_depth: f32,
  /// Mean annual temperature at the lip, in °C.
  pub celsius: f32,
  /// A trickle (under [`TRICKLE_DISCHARGE`] and [`TRICKLE_WIDTH`]): its
  /// step is whitewater on the river ribbon, with no sheet, mist or pool.
  pub trickle: bool,
  /// For a cascade, its steps from the top; empty for a single fall.
  pub steps: Vec<FallStep>,
}

impl Fall {
  /// Height of the drop, in metres: for a cascade, the total drop.
  pub fn height(&self) -> f32 {
    self.lip_level - self.foot_level
  }
}

/// Falls carrying less than this, in cubic metres per second, and
/// narrower than [`TRICKLE_WIDTH`], are trickles.
pub const TRICKLE_DISCHARGE: f32 = 0.05;

/// See [`TRICKLE_DISCHARGE`], in metres.
pub const TRICKLE_WIDTH: f32 = 1.0;

/// Plunge pool radius and depth, in metres, under a fall of `height`
/// metres landing in a channel `width` metres wide with `discharge` m³/s:
/// `0.3 height + width` and `0.15 height`, both scaled by
/// `clamp(sqrt(Q) / 2, 0.15, 1)`, so a trickle does not dig a pool the size
/// of a river's. The depth is at least 0.3 m.
pub fn pool_size(height: f32, width: f32, discharge: f32) -> (f32, f32) {
  let scale = (discharge.max(0.0).sqrt() / 2.0).clamp(0.15, 1.0);
  (
    (0.3 * height + width) * scale,
    (0.15 * height * scale).max(0.3),
  )
}

/// A cut-off meander loop holding still water.
#[derive(Clone, Debug, PartialEq)]
pub struct Oxbow {
  /// Centreline, in heightmap sample coordinates.
  pub points: Vec<[f32; 2]>,
  /// Width in metres.
  pub width: f32,
  /// Water surface height in metres.
  pub surface: f32,
  /// Depth of its hollow below the surface in metres: older oxbows have
  /// silted up and are shallower.
  pub depth: f32,
  /// Mean annual temperature in °C.
  pub celsius: f32,
}

/// A stream ready for conditioning, from the drainage or a painted mask.
#[derive(Clone, Debug, PartialEq)]
pub struct RawStream {
  /// Heightmap sample coordinates, upstream to downstream.
  pub points: Vec<[f32; 2]>,
  /// Water level at each point before conditioning.
  pub levels: Vec<f32>,
  /// Discharge at each point.
  pub discharge: Vec<f32>,
  /// Strahler order at each point.
  pub orders: Vec<u8>,
  /// Channel width at least this, in metres (painted rivers).
  pub min_width: f32,
  /// Where it ends.
  pub mouth: Mouth,
  /// Painted by the author: its centreline is kept exactly as painted.
  pub painted: bool,
}

/// Everything the channel stage produces.
#[derive(Clone, Debug, Default)]
pub struct Channels {
  /// Channel reaches, including delta distributaries.
  pub reaches: Vec<Reach>,
  /// Waterfalls.
  pub falls: Vec<Fall>,
  /// Oxbow lakes.
  pub oxbows: Vec<Oxbow>,
  /// Full-resolution mask of samples under a channel.
  pub mask: Vec<bool>,
}

/// Inputs shared by every stream.
pub struct ChannelContext<'a> {
  /// Surface samples before rivers were carved (may be empty).
  pub surface: &'a [SurfaceSample],
  /// River options.
  pub options: &'a RiverOptions,
  /// Seeds meander phases and delta splits.
  pub seed: u64,
}

/// Turn a hydrology's drainage streams into raw streams.
pub fn raw_streams(hydrology: &Hydrology) -> Vec<RawStream> {
  hydrology
    .streams()
    .into_iter()
    .map(|stream| {
      let n = stream.cells.len();
      let mut discharge: Vec<f32> = stream
        .cells
        .iter()
        .map(|cell| hydrology.discharge[*cell as usize])
        .collect();

      // A sea or lake cell carries everything that reaches it, and a
      // join cell everything its main stem carries; the stream's own
      // discharge is the last cell above it. Otherwise its width would
      // swell to the main stem's over its last stretch into a funnel.
      if n >= 2 {
        discharge[n - 1] = discharge[n - 2];
      }

      let levels = stream
        .cells
        .iter()
        .enumerate()
        .map(|(i, cell)| match stream.mouth {
          Mouth::Sea if i == n - 1 => hydrology.sea,
          Mouth::Lake(id) if i == n - 1 => hydrology.lakes[id as usize].surface,
          _ => hydrology.filled[*cell as usize],
        })
        .collect();

      RawStream {
        points: stream
          .cells
          .iter()
          .map(|cell| {
            let (x, y) = hydrology.sample_xy(*cell);
            [x as f32, y as f32]
          })
          .collect(),
        levels,
        discharge,
        orders: {
          let mut orders: Vec<u8> = stream
            .cells
            .iter()
            .map(|cell| hydrology.strahler[*cell as usize])
            .collect();

          // The join cell has the main stem's order.
          if stream.mouth == Mouth::Join && n >= 2 {
            orders[n - 1] = orders[n - 2];
          }

          orders
        },
        min_width: 0.0,
        mouth: stream.mouth,
        painted: false,
      }
    })
    .collect()
}

/// Bilinear height at fractional sample coordinates.
pub fn height_at(map: &HeightMap, x: f32, y: f32) -> f32 {
  let width = map.metadata.width;
  let height = map.metadata.height;
  let fx = x.clamp(0.0, (width - 1) as f32);
  let fy = y.clamp(0.0, (height - 1) as f32);
  let x0 = (fx as u32).min(width.saturating_sub(2));
  let y0 = (fy as u32).min(height.saturating_sub(2));
  let x1 = (x0 + 1).min(width - 1);
  let y1 = (y0 + 1).min(height - 1);
  let tx = fx - x0 as f32;
  let ty = fy - y0 as f32;
  let at = |x: u32, y: u32| map.heights[(y * width + x) as usize];
  let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
  let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
  top + (bottom - top) * ty
}

fn surface_at<'a>(
  surface: &'a [SurfaceSample],
  map: &HeightMap,
  x: f32,
  y: f32,
) -> Option<&'a SurfaceSample> {
  let width = map.metadata.width;
  let sx = (x.round().max(0.0) as u32).min(width - 1);
  let sy = (y.round().max(0.0) as u32).min(map.metadata.height - 1);
  surface.get((sy * width + sx) as usize)
}

/// How reaches are cut: once per sample, or (in tests) with the reference
/// carve it replaced.
enum Carver {
  Once(CarveScratch),
  #[cfg(test)]
  Reference,
}

/// Shape every stream's bed and banks into `map`, recording every change
/// in `record`. Streams must be ordered main stems first, so tributaries
/// meet them at their level.
pub fn condition_channels(
  map: &mut HeightMap,
  hydrology: &Hydrology,
  streams: Vec<RawStream>,
  context: &ChannelContext<'_>,
  record: &mut CarveRecord,
) -> Channels {
  let scratch = CarveScratch::new(map);
  condition_with(
    map,
    hydrology,
    streams,
    context,
    record,
    Carver::Once(scratch),
  )
}

fn condition_with(
  map: &mut HeightMap,
  hydrology: &Hydrology,
  streams: Vec<RawStream>,
  context: &ChannelContext<'_>,
  record: &mut CarveRecord,
  mut carver: Carver,
) -> Channels {
  let mut channels = Channels {
    mask: vec![false; map.heights.len()],
    ..Channels::default()
  };
  let metres = map.metadata.metres_per_sample.max(0.001);
  // The stream that first claimed each sample, and each stream's reaches,
  // so a tributary finds the curve of the stream it joins.
  let mut owner = vec![u32::MAX; map.heights.len()];
  let mut owned: Vec<std::ops::Range<usize>> = Vec::new();
  // And the loops each stream cut off as it migrated.
  let mut cut: Vec<Vec<CutLoop>> = Vec::new();
  // Scratch for every stream's belt, cleared after each.
  let mut near = Near {
    flags: vec![false; map.heights.len()],
    set: Vec::new(),
  };
  let map_width = map.metadata.width;
  let map_height = map.metadata.height;
  let slot = |x: f32, y: f32| {
    let x = (x.round().max(0.0) as u32).min(map_width - 1);
    let y = (y.round().max(0.0) as u32).min(map_height - 1);
    (y * map_width + x) as usize
  };

  for (index, mut raw) in streams.into_iter().enumerate() {
    if raw.points.len() < 2 {
      continue;
    }

    let mut join = None;

    if raw.mouth == Mouth::Join {
      if let Some(stream) = owner_of(&owner, map, raw.points[raw.points.len() - 1]) {
        let main = &channels.reaches[owned[stream].clone()];

        // The main stem may have migrated across the tributary's path:
        // it ends where it first reaches the main stem's water.
        if !raw.painted {
          let end = first_contact(&raw.points, main, metres) + 1;
          raw.points.truncate(end);
          raw.levels.truncate(end);
          raw.discharge.truncate(end);
          raw.orders.truncate(end);
        }

        join = join_on(main, &cut[stream], raw.points[raw.points.len() - 1]);
      }

      if let Some(Join { point, .. }) = join {
        let n = raw.points.len();
        raw.levels[n - 1] = raw.levels[n - 1].min(point.level);

        // Painted rivers keep the path their author drew.
        if !raw.painted {
          raw.points[n - 1] = [point.x, point.y];
        }
      }
    }

    let seed = hash_u64(context.seed ^ (index as u64).wrapping_mul(0x2545_f491_4f6c_dd1d));
    let Shaped {
      mut reaches,
      falls,
      oxbows,
      loops,
      lines,
      fan,
    } = shape_stream(&raw, join.as_ref(), map, metres, context, seed);

    for (sample, target) in fan {
      if target > map.heights[sample] {
        record.set(map, sample, target);

        match &mut carver {
          Carver::Once(scratch) => scratch.raised(sample, map_width as usize, target),
          #[cfg(test)]
          Carver::Reference => {}
        }
      }
    }

    let number = owned.len() as u32;
    owned.push(channels.reaches.len()..channels.reaches.len() + reaches.len());
    cut.push(loops);

    for point in &raw.points {
      let at = slot(point[0], point[1]);

      if owner[at] == u32::MAX {
        owner[at] = number;
      }
    }

    // Pools first, so the channel below cuts its outlet through the lip.
    // A cascade's upper steps leave only churned water, and trickles none.
    for fall in falls.iter().filter(|fall| !fall.trickle) {
      carve_pool(map, fall, metres, record, &mut channels.mask);
    }

    for reach in &reaches {
      // The dense points are for drawing; the carve needs only the
      // sample-scale shape, which keeps it as quick as the cell path.
      let keep: Vec<bool> = reach
        .points
        .iter()
        .map(|point| point.falling || point.rapids > 0.0)
        .collect();
      let reach = &Reach {
        points: simplify(&reach.points, &keep, 0.1),
        ..Reach::default()
      };

      match &mut carver {
        Carver::Once(scratch) => carve_reach(
          map,
          hydrology,
          context.surface,
          reach,
          metres,
          record,
          &mut channels.mask,
          scratch,
        ),
        #[cfg(test)]
        Carver::Reference => carve_reach_reference(
          map,
          hydrology,
          context.surface,
          reach,
          metres,
          record,
          &mut channels.mask,
        ),
      }
    }

    carve_belt(map, &lines, metres, record, &mut near);

    for oxbow in &oxbows {
      carve_oxbow(map, oxbow, metres, record);
    }

    channels.reaches.append(&mut reaches);
    channels.falls.extend(falls);
    channels.oxbows.extend(oxbows);
  }

  channels
}

/// Where a tributary meets the stream it joins: the point of its
/// centreline, and that stream's downstream direction there.
#[derive(Clone, Copy)]
struct Join {
  point: ChannelPoint,
  along: [f32; 2],
}

/// The stream that claimed `last`'s sample, or one next to it.
fn owner_of(owner: &[u32], map: &HeightMap, last: [f32; 2]) -> Option<usize> {
  let (width, height) = (map.metadata.width as i32, map.metadata.height as i32);
  let (cx, cy) = (last[0].round() as i32, last[1].round() as i32);
  (-1..=1)
    .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
    .map(|(dx, dy)| (cx + dx, cy + dy))
    .filter(|(x, y)| *x >= 0 && *y >= 0 && *x < width && *y < height)
    .map(|(x, y)| owner[(y * width + x) as usize])
    .find(|number| *number != u32::MAX)
    .map(|number| number as usize)
}

/// The nearest point of `reaches` to `target`, with its distance in
/// samples.
fn nearest(reaches: &[Reach], target: [f32; 2]) -> Option<(f32, &[ChannelPoint], usize)> {
  let mut best: Option<(f32, &[ChannelPoint], usize)> = None;

  for reach in reaches {
    for (k, p) in reach.points.iter().enumerate() {
      let distance = length2(p.x - target[0], p.y - target[1]);

      if best.is_none_or(|(d, _, _)| distance < d) {
        best = Some((distance, &reach.points, k));
      }
    }
  }

  best
}

/// Where a tributary ending at `last` meets its main stem: the nearest
/// point of the main stem's drawn centreline. Where the main stem cut off
/// the loop the tributary ended on, the tributary joins where that loop
/// left the channel.
fn join_on(main: &[Reach], loops: &[CutLoop], last: [f32; 2]) -> Option<Join> {
  let (distance, _, _) = nearest(main, last)?;
  let target = loops
    .iter()
    .flat_map(|cut| cut.points.iter().map(move |p| (p, cut.connection)))
    .map(|(p, connection)| (length2(p.x - last[0], p.y - last[1]), connection))
    .filter(|(d, _)| *d < distance)
    .min_by(|a, b| a.0.total_cmp(&b.0))
    .map_or(last, |(_, connection)| connection);
  let (_, points, k) = nearest(main, target)?;
  let (a, b) = (
    &points[k.saturating_sub(1)],
    &points[(k + 1).min(points.len() - 1)],
  );
  let length = length2(b.x - a.x, b.y - a.y).max(1e-6);
  Some(Join {
    point: points[k],
    along: [(b.x - a.x) / length, (b.y - a.y) / length],
  })
}

/// The last point a tributary keeps: the end of the first step of its
/// path that comes within half the main stem's width (plus half a sample,
/// for the smoothing) of the main stem's centreline, or its last point.
/// Steps, not points, are tested: a diagonal step can straddle a narrow
/// channel. Only the main stem within 24 widths of the join is searched:
/// that is as far as its meander belt reaches.
fn first_contact(points: &[[f32; 2]], main: &[Reach], metres: f32) -> usize {
  let n = points.len();
  let end = points[n - 1];
  let near: Vec<&ChannelPoint> = main
    .iter()
    .flat_map(|reach| &reach.points)
    .filter(|q| length2(q.x - end[0], q.y - end[1]) * metres <= 24.0 * q.width)
    .collect();
  let reach = near
    .iter()
    .fold(0.0f32, |m, q| m.max(24.0 * q.width / metres + 2.0));
  let distance = |q: &ChannelPoint, a: [f32; 2], b: [f32; 2]| {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let t =
      (((q.x - a[0]) * dx + (q.y - a[1]) * dy) / (dx * dx + dy * dy).max(1e-9)).clamp(0.0, 1.0);
    length2(a[0] + t * dx - q.x, a[1] + t * dy - q.y)
  };

  (0..n.saturating_sub(2))
    .find(|i| {
      let (a, b) = (points[*i], points[i + 1]);
      length2(a[0] - end[0], a[1] - end[1]) <= reach
        && near
          .iter()
          .any(|q| distance(q, a, b) * metres < 0.5 * q.width + 0.5 * metres)
    })
    .map_or(n - 1, |i| i + 1)
}

/// Bend a tributary's end into its main stem at the angle real junctions
/// take: acute and pointing downstream, wider where the tributary is much
/// steeper than the main stem (Horton, 1945; Howard, 1971), `acos(S_main /
/// S_trib)` within 25 to 85 degrees. The last `L = clamp(3 w_main, 1, 4)`
/// samples become a cubic Hermite curve from the point `L` upstream, along
/// its own direction, to the join, arriving along the main stem's
/// direction turned by that angle towards the tributary's side. Where the
/// curve would climb onto ground the channel may not, or bend tighter than
/// the rest of the stream, `2 L`, `L / 2` and `L / 4` are tried, then the
/// same with the angle widened towards a right angle; failing all of
/// them, the smoothed path stays.
fn junction(points: &mut Vec<ChannelPoint>, join: &Join, map: &HeightMap, metres: f32) {
  let n = points.len();

  if n < 4 {
    return;
  }

  let end = points[n - 1];
  let main = join.point;
  // The tributary's slope over its last 3 w.
  let mut k = n - 1;

  while k > 0 && length2(end.x - points[k].x, end.y - points[k].y) * metres < 3.0 * end.width {
    k -= 1;
  }

  let run = length2(end.x - points[k].x, end.y - points[k].y) * metres;
  let steep = (points[k].level - end.level) / run.max(1e-3);
  let angle = (main.slope / steep.max(1e-6)).clamp(0.087, 0.906).acos();
  let reach = (3.0 * main.width / metres).clamp(1.0, 4.0);
  // The curve must bend no tighter than the rest of the stream; a short
  // chord cannot turn far enough, so try a longer one before shorter ones.
  let least = end.width.max(0.5 * metres) / metres;

  // Where the tributary arrives facing upstream (a migrated main stem
  // can swing its bend round), no chord turns it that far: widen the
  // angle towards a right angle before giving up.
  let widened = [angle, 0.5 * (angle + FRAC_PI_2), FRAC_PI_2];
  let tries = widened
    .iter()
    .flat_map(|angle| [1.0, 2.0, 0.5, 0.25].map(|scale| (*angle, scale * reach)));

  for (angle, reach) in tries {
    // The first point at least `reach` samples back along the stream.
    let mut from = n - 1;
    let mut walked = 0.0;

    while from > 0 && walked < reach {
      walked += length2(
        points[from].x - points[from - 1].x,
        points[from].y - points[from - 1].y,
      );
      from -= 1;
    }

    if from == 0 || points[from..].iter().any(|p| p.falling || p.rapids > 0.0) {
      continue;
    }

    let start = points[from];
    let before = points[from - 1];
    let chord = length2(end.x - start.x, end.y - start.y);
    let lead = length2(start.x - before.x, start.y - before.y).max(1e-6);
    let t0 = [
      (start.x - before.x) / lead * chord,
      (start.y - before.y) / lead * chord,
    ];
    // Towards the tributary's side of the main stem.
    let side = join.along[0] * (start.y - end.y) - join.along[1] * (start.x - end.x);
    let turn = -angle * side.signum();
    let (sin, cos) = turn.sin_cos();
    let arrive = [
      (join.along[0] * cos - join.along[1] * sin) * chord,
      (join.along[0] * sin + join.along[1] * cos) * chord,
    ];
    // A cubic whose start tangent lies near its chord puts most of its
    // turn at the far end, up to five times the average, so sample about
    // 6 degrees of turn a step to keep the sharpest step under 30 degrees.
    let heading = (t0[0] * arrive[1] - t0[1] * arrive[0])
      .abs()
      .atan2(t0[0] * arrive[0] + t0[1] * arrive[1]);
    let count = ((walked * metres) / crate::terrain::centreline::spacing(end.width, metres))
      .ceil()
      .max((heading / 0.1).ceil() + 1.0)
      .max(2.0) as usize;
    let curve: Vec<ChannelPoint> = (1..=count)
      .map(|j| {
        let t = j as f32 / count as f32;
        let (t2, t3) = (t * t, t * t * t);
        let h = [
          2.0 * t3 - 3.0 * t2 + 1.0,
          t3 - 2.0 * t2 + t,
          3.0 * t2 - 2.0 * t3,
          t3 - t2,
        ];
        let at = [
          h[0] * start.x + h[1] * t0[0] + h[2] * end.x + h[3] * arrive[0],
          h[0] * start.y + h[1] * t0[1] + h[2] * end.y + h[3] * arrive[1],
        ];
        crate::terrain::centreline::blend(&start, &end, t, at)
      })
      .collect();

    let xy = |p: &ChannelPoint| [p.x, p.y];
    let joined: Vec<[f32; 2]> = std::iter::once(xy(&start))
      .chain(curve.iter().map(xy))
      .collect();
    let smooth = joined
      .windows(3)
      .all(|w| crate::terrain::centreline::radius(w[0], w[1], w[2]) >= least);

    if smooth
      && curve[..count - 1]
        .iter()
        .all(|p| crate::terrain::centreline::ground_allows(map, p.x, p.y, p, metres))
    {
      points.truncate(from + 1);
      points.extend(curve);
      return;
    }
  }
}

/// Bedrock channels on steep ground are narrower: the width times
/// `clamp((S / 0.02)^(-3/16), 0.7, 1)` over 2 %, the slope term of
/// Finnegan et al. (2005).
pub fn narrowing(slope: f32) -> f32 {
  if slope > 0.02 {
    (slope / 0.02).powf(-3.0 / 16.0).max(0.7)
  } else {
    1.0
  }
}

/// Flare a river mouth into the funnel of an alluvial estuary, whose width
/// grows exponentially seawards (Savenije, 2005): over the last `L_e` of
/// the river, the channel within 3 m of sea level (at most 20 w0, with w0
/// the width where the flare starts), the width is `w0 x 3^(1 - x / L_e)`
/// at `x` from the mouth, 3 w0 at the sea. On a steep coast, where `L_e` is
/// under 2 w0, the mouth widens to 1.5 w0 at most. Its banks open at no
/// more than 12 degrees where the flare starts, so a short one widens
/// less, and they curve out smoothly from there. Returns whether it is a
/// wide estuary (3 w0, or as near it as its banks allow).
fn estuary(points: &mut [ChannelPoint], sea: f32, metres: f32) -> bool {
  let n = points.len();
  let mut k = n - 1;
  let mut x = 0.0;

  while k > 0 {
    let (p, q) = (&points[k - 1], &points[k]);
    let next = x + length2(q.x - p.x, q.y - p.y) * metres;

    if p.level - sea > 3.0 || next > 20.0 * p.width {
      break;
    }

    x = next;
    k -= 1;
  }

  let (w0, reach) = (points[k].width, x);

  if reach <= 0.0 {
    return false;
  }

  // A bank opens at `atan(w0 ln(most) / (2 L_e))` where the flare starts:
  // 12 degrees at most.
  let wide = reach >= 2.0 * w0;
  let most = if wide { 3.0f32 } else { 1.5 }.min((0.425 * reach / w0).exp());
  let mut x = reach;

  for i in k + 1..n {
    x -= length2(points[i].x - points[i - 1].x, points[i].y - points[i - 1].y) * metres;
    points[i].width = points[i]
      .width
      .max(w0 * most.powf(1.0 - x.max(0.0) / reach));
  }

  wide
}

/// Mouth bars: two shoals of sand just above the sea flanking a wide
/// estuary's mouth (`points` ends at the sea), from its sides out to a
/// mouth-width seawards, so the river meets a beach rather than the sea
/// meeting a funnel. Returns the samples raised and their new heights, as
/// a delta's fan does.
fn mouth_bars(points: &[ChannelPoint], map: &HeightMap, metres: f32) -> Vec<(usize, f32)> {
  let n = points.len();
  let mut raised = Vec::new();

  if n < 2 {
    return raised;
  }

  let (p, q) = (&points[n - 1], &points[n - 2]);
  let length = length2(p.x - q.x, p.y - q.y).max(1e-6);
  let toward = [(p.x - q.x) / length, (p.y - q.y) / length];
  // In samples.
  let width = p.width / metres;

  if width < 2.0 {
    return raised;
  }

  let sea = map.metadata.sea_level_metres;
  let (map_width, map_height) = (map.metadata.width as i32, map.metadata.height as i32);
  let r = (1.5 * width).ceil() as i32;

  for y in (p.y as i32 - r).max(0)..=(p.y as i32 + r).min(map_height - 1) {
    for x in (p.x as i32 - r).max(0)..=(p.x as i32 + r).min(map_width - 1) {
      let (dx, dy) = (x as f32 - p.x, y as f32 - p.y);
      let along = dx * toward[0] + dy * toward[1];
      let across = (dx * toward[1] - dy * toward[0]).abs();
      // An ellipse on each side: 0 at its spine, 1 at its rim.
      let e = ((along - 0.4 * width) / (0.7 * width)).powi(2)
        + ((across - 0.85 * width) / (0.3 * width)).powi(2);
      let index = (y * map_width + x) as usize;
      let target = sea + 0.3 - 0.8 * e;

      if e <= 1.0 && !map.no_data[index] && map.heights[index] < target {
        raised.push((index, target));
      }
    }
  }

  raised
}

/// Arc length along a polyline in metres, at each point.
pub(crate) fn arc_lengths(points: &[[f32; 2]], metres: f32) -> Vec<f32> {
  let mut s = Vec::with_capacity(points.len());
  let mut total = 0.0;

  for (i, p) in points.iter().enumerate() {
    if i > 0 {
      let q = points[i - 1];
      total += length2(p[0] - q[0], p[1] - q[1]) * metres;
    }

    s.push(total);
  }

  s
}

/// Find waterfall steps on a level profile: segments steeper than 35
/// degrees, or dropping more than `max(3 m, 1.5 w)` within two samples far
/// more steeply than the reach around them, grouped into steps of at most
/// two samples. Returns point index ranges `(lip, foot)`; `rapids` gets
/// the steps lower than 3 m.
pub fn find_steps(
  levels: &[f32],
  s: &[f32],
  widths: &[f32],
  rapids: &mut [f32],
) -> Vec<(usize, usize)> {
  let n = levels.len();
  let mut flagged = vec![false; n.saturating_sub(1)];

  for i in 0..n.saturating_sub(1) {
    let ds = (s[i + 1] - s[i]).max(0.01);
    let gradient = (levels[i] - levels[i + 1]) / ds;
    let two = levels[i] - levels[(i + 2).min(n - 1)];
    let a = i.saturating_sub(10);
    let b = (i + 11).min(n - 1);
    let background = (levels[a] - levels[b]) / (s[b] - s[a]).max(0.01);
    flagged[i] = gradient > 0.7
      || (two > MIN_FALL_METRES.max(1.5 * widths[i])
        && gradient > 0.1
        && gradient > background * 3.0);
  }

  // A step spans at most two samples, so a long steep run becomes a
  // staircase of falls and pools, as steep mountain streams are.
  let mut steps = Vec::new();
  let mut i = 0;

  while i < flagged.len() {
    if !flagged[i] {
      i += 1;
      continue;
    }

    let start = i;

    while i < flagged.len() && flagged[i] && i - start < 2 {
      i += 1;
    }

    // Points start..=i span the step.
    if levels[start] - levels[i] >= MIN_FALL_METRES {
      steps.push((start, i));
    } else {
      for value in &mut rapids[start..=i] {
        *value = 1.0;
      }
    }
  }

  steps
}

/// Level profile that never rises: each point at most its upstream
/// neighbour, then a 5-point average that also never rises, leaving the
/// points of each step untouched.
pub fn condition_profile(levels: &mut [f32], steps: &[(usize, usize)]) {
  for i in 1..levels.len() {
    levels[i] = levels[i].min(levels[i - 1]);
  }

  let n = levels.len();
  let mut fixed = vec![false; n];
  fixed[0] = true;
  fixed[n - 1] = true;

  for (a, b) in steps {
    for value in &mut fixed[*a..=*b] {
      *value = true;
    }
  }

  let original = levels.to_vec();

  for i in 1..n {
    if fixed[i] {
      levels[i] = levels[i].min(levels[i - 1]);
      continue;
    }

    // Average within the stretch between steps.
    let mut sum = 0.0;
    let mut count = 0.0;

    let first = i.saturating_sub(2);

    for (j, value) in original
      .iter()
      .enumerate()
      .take((i + 2).min(n - 1) + 1)
      .skip(first)
    {
      let blocked = (j.min(i)..j.max(i)).any(|k| fixed[k] && k != 0 && k != i && k != j);

      if !blocked {
        sum += value;
        count += 1.0;
      }
    }

    levels[i] = (sum / count).min(levels[i - 1]).min(original[i]);
  }
}

/// A normalised lateral offset curve for one meander wavelength, from a
/// Kinoshita curve (a sine-generated curve with skew and flattening
/// terms), sampled at 64 points along the valley.
pub(crate) fn kinoshita_table() -> [f32; 64] {
  let theta0 = 1.4f32;
  let skew = 1.0 / 32.0;
  let flat = 1.0 / 192.0;
  let steps = 512;
  let mut xs = Vec::with_capacity(steps + 1);
  let mut ys = Vec::with_capacity(steps + 1);
  let (mut x, mut y) = (0.0f32, 0.0f32);

  for k in 0..=steps {
    xs.push(x);
    ys.push(y);
    let u = k as f32 / steps as f32;
    let tau = std::f32::consts::TAU;
    let theta = theta0
      * ((tau * u).sin()
        + theta0 * theta0 * (skew * (3.0 * tau * u).cos() - flat * (3.0 * tau * u).sin()));
    x += theta.cos() / steps as f32;
    y += theta.sin() / steps as f32;
  }

  let total = xs[steps];
  let drift = ys[steps];
  let mut table = [0.0f32; 64];
  let mut k = 0;

  for (slot, value) in table.iter_mut().enumerate() {
    let target = slot as f32 / 64.0 * total;

    while k + 1 < steps && xs[k + 1] < target {
      k += 1;
    }

    let t = ((target - xs[k]) / (xs[k + 1] - xs[k]).max(1e-6)).clamp(0.0, 1.0);
    let along = xs[k] + (xs[k + 1] - xs[k]) * t;
    *value = ys[k] + (ys[k + 1] - ys[k]) * t - drift * along / total;
  }

  let mean = table.iter().sum::<f32>() / 64.0;
  let peak = table
    .iter()
    .map(|v| (v - mean).abs())
    .fold(0.0f32, f32::max)
    .max(1e-6);

  for value in &mut table {
    *value = (*value - mean) / peak;
  }

  table
}

pub(crate) fn kinoshita(table: &[f32; 64], phase: f32) -> f32 {
  let p = phase.rem_euclid(1.0) * 64.0;
  let i = p as usize % 64;
  let t = p - p.floor();
  table[i] + (table[(i + 1) % 64] - table[i]) * t
}

/// One conditioned stream.
#[derive(Default)]
struct Shaped {
  /// One reach, or a trunk and distributaries for a delta.
  reaches: Vec<Reach>,
  falls: Vec<Fall>,
  oxbows: Vec<Oxbow>,
  /// Loops cut off as the stream migrated, so tributaries that joined
  /// them find the channel again.
  loops: Vec<CutLoop>,
  /// A migrated stream's final centreline, then its snapshots, for the
  /// meander belt's floor and scroll bars; empty for other streams.
  lines: Vec<Vec<ChannelPoint>>,
  /// Delta fan or mouth bar samples and the heights they are raised to.
  fan: Vec<(usize, f32)>,
}

/// Condition one stream.
fn shape_stream(
  raw: &RawStream,
  join: Option<&Join>,
  map: &HeightMap,
  metres: f32,
  context: &ChannelContext<'_>,
  seed: u64,
) -> Shaped {
  let options = context.options;
  let width_scale = options.width_scale.clamp(0.1, 10.0);
  let n = raw.points.len();
  let s = arc_lengths(&raw.points, metres);
  let widths: Vec<f32> = raw
    .discharge
    .iter()
    .map(|q| channel_width(*q, width_scale).max(raw.min_width))
    .collect();
  let mut levels = raw.levels.clone();
  let mut rapids = vec![0.0; n];

  for i in 1..n {
    levels[i] = levels[i].min(levels[i - 1]);
  }

  let steps = if options.waterfalls {
    find_steps(&levels, &s, &widths, &mut rapids)
  } else {
    Vec::new()
  };
  condition_profile(&mut levels, &steps);

  // Trickles are drawn as whitewater down their step, so their drop
  // counts towards the slope, and only real falls are left out of it.
  let trickle =
    |(a, _): &(usize, usize)| raw.discharge[*a] < TRICKLE_DISCHARGE && widths[*a] < TRICKLE_WIDTH;
  let falling: Vec<(usize, usize)> = steps
    .iter()
    .copied()
    .filter(|step| !trickle(step))
    .collect();
  let in_step = |i: usize| falling.iter().any(|(a, b)| i >= *a && i < *b);

  for step in steps.iter().filter(|step| trickle(step)) {
    for value in &mut rapids[step.0..=step.1] {
      *value = 1.0;
    }
  }

  let celsius =
    |x: f32, y: f32| surface_at(context.surface, map, x, y).map_or(15.0, |s| s.celsius());
  let mut points: Vec<ChannelPoint> = (0..n)
    .map(|i| {
      // Valley slope over about three points each way, leaving out falls.
      let (mut drop, mut run) = (0.0, 0.0);

      for k in i.saturating_sub(3)..(i + 3).min(n - 1) {
        if !in_step(k) {
          drop += levels[k] - levels[k + 1];
          run += s[k + 1] - s[k];
        }
      }

      let slope = if run > 0.0 {
        (drop / run).max(0.0)
      } else {
        0.0
      };
      let depth = channel_depth(raw.discharge[i]);
      let slope = slope.max(if rapids[i] > 0.0 { 0.05 } else { 0.0 });

      ChannelPoint {
        x: raw.points[i][0],
        y: raw.points[i][1],
        level: levels[i],
        bed: levels[i] - depth,
        width: (channel_width(raw.discharge[i], width_scale) * narrowing(slope)).max(raw.min_width),
        depth,
        discharge: raw.discharge[i],
        slope,
        speed: manning_speed(depth, slope),
        curvature: 0.0,
        celsius: celsius(raw.points[i][0], raw.points[i][1]),
        rapids: rapids[i],
        falling: false,
        order: raw.orders.get(i).copied().unwrap_or(0),
      }
    })
    .collect();

  // Falls whose foot lies within three pool radii of the next lip form
  // one cascade, with one sheet over its steps and one pool at the bottom.
  let pool_radius = |(a, b): (usize, usize)| {
    let (lip, foot) = (&points[a], &points[b]);
    pool_size(lip.level - foot.level, foot.width, lip.discharge).0
  };
  let mut groups: Vec<Vec<(usize, usize)>> = Vec::new();

  for step in &falling {
    match groups.last_mut() {
      Some(group)
        if {
          let last = group[group.len() - 1];
          s[step.0] - s[last.1] <= 3.0 * pool_radius(last)
        } =>
      {
        group.push(*step)
      }
      _ => groups.push(vec![*step]),
    }
  }

  for group in &groups {
    for point in &mut points[group[0].0..=group[group.len() - 1].1] {
      point.falling = true;
    }
  }

  // The cell path becomes a smooth curve along the valley. Heads, mouths
  // and points near the map edge stay where they are; both ends of every
  // step stay points of the curve, and the falls are placed on it after.
  let mut stepped = vec![false; n];

  for (a, b) in &steps {
    stepped[*a] = true;
    stepped[*b] = true;
  }

  let (points, stepped, at) = if raw.painted {
    (points, stepped, (0..n).collect::<Vec<usize>>())
  } else {
    let edge = |p: &ChannelPoint| {
      p.x < 1.0
        || p.y < 1.0
        || p.x > (map.metadata.width - 2) as f32
        || p.y > (map.metadata.height - 2) as f32
    };
    // A tributary's last stretch stays on its cells: smoothing would pull
    // it into the main stem's carved bed, and the junction curve replaces
    // it anyway.
    let near_join = |p: &ChannelPoint| {
      join.is_some_and(|join| {
        length2(p.x - join.point.x, p.y - join.point.y) <= 0.5 * join.point.width / metres + 1.0
      })
    };
    let fixed: Vec<bool> = points.iter().map(|p| edge(p) || near_join(p)).collect();
    let smoothed = continuous_centreline(&points, &fixed, &stepped, map, metres);
    // Where each kept point went: they stay in order.
    let mut at = vec![0; n];
    let mut kept = smoothed
      .pins
      .iter()
      .enumerate()
      .filter(|(_, pin)| **pin)
      .map(|(k, _)| k);
    let mut new_stepped = vec![false; smoothed.points.len()];

    for i in (0..n).filter(|i| fixed[*i] || stepped[*i] || *i == 0 || *i == n - 1) {
      at[i] = kept.next().unwrap_or(0);
      new_stepped[at[i]] = stepped[i];
    }

    (smoothed.points, new_stepped, at)
  };
  let step_of = |(a, b): (usize, usize)| {
    let (lip, foot) = (&points[at[a]], &points[at[b]]);
    let height = lip.level - foot.level;
    FallStep {
      lip: [lip.x, lip.y],
      lip_level: lip.level,
      foot: [foot.x, foot.y],
      foot_level: foot.level,
      pool_radius: pool_size(height, foot.width, lip.discharge).0,
    }
  };
  let fall = |(a, b): (usize, usize), steps: Vec<FallStep>, trickle: bool| {
    let (a, b) = (at[a], at[b]);
    let lip = points[a];
    let foot = points[b];
    let dx = foot.x - lip.x;
    let dy = foot.y - lip.y;
    let length = length2(dx, dy).max(1e-4);
    let (pool_radius, pool_depth) = if trickle {
      (0.0, 0.0)
    } else {
      pool_size(lip.level - foot.level, foot.width, lip.discharge)
    };

    Fall {
      lip: [lip.x, lip.y],
      lip_level: lip.level,
      foot: [foot.x, foot.y],
      foot_level: foot.level,
      direction: [dx / length, dy / length],
      width: lip.width,
      discharge: lip.discharge,
      // The water arrives at the lip at the speed of the reach above.
      speed: points[a.saturating_sub(1)].speed,
      pool_radius,
      pool_depth,
      celsius: lip.celsius,
      trickle,
      steps,
    }
  };
  let falls: Vec<Fall> = groups
    .iter()
    .map(|group| {
      let span = (group[0].0, group[group.len() - 1].1);
      let steps = if group.len() > 1 {
        group.iter().map(|step| step_of(*step)).collect()
      } else {
        Vec::new()
      };
      fall(span, steps, false)
    })
    .chain(
      steps
        .iter()
        .filter(|step| trickle(step))
        .map(|step| fall(*step, Vec::new(), true)),
    )
    .collect();
  let mut points = points;
  let mut loops = Vec::new();
  let mut lines = Vec::new();

  // Wide reaches on gentle open ground migrate into meanders. Heads,
  // mouths, step ends and the map edge stay where they are.
  if options.meanders > 0.0 && !raw.painted && points.iter().any(|p| representable(p.width, metres))
  {
    let n = points.len();
    let (right, bottom) = (
      (map.metadata.width - 2) as f32,
      (map.metadata.height - 2) as f32,
    );
    let pins: Vec<bool> = points
      .iter()
      .enumerate()
      .map(|(i, p)| {
        i == 0 || i == n - 1 || stepped[i] || p.x < 1.0 || p.y < 1.0 || p.x > right || p.y > bottom
      })
      .collect();
    let floors = floor_widths(&points, map, metres);
    let migrated = migrate(
      &points,
      &pins,
      &floors,
      &Migration {
        map,
        metres,
        strength: options.meanders.clamp(0.0, 1.0),
        maturity: options.meander_maturity.clamp(0.0, 1.0),
        seed,
        sea_mouth: raw.mouth == Mouth::Sea,
      },
    );
    points = migrated.points;
    loops = migrated.cut;

    if !migrated.snapshots.is_empty() {
      lines = migrated.snapshots;
      lines.insert(0, points.clone());
    }
  }

  let oxbows: Vec<Oxbow> = loops.iter().filter_map(oxbow).collect();

  if let (Some(join), false) = (join, raw.painted) {
    junction(&mut points, join, map, metres);
  }

  if raw.mouth == Mouth::Sea {
    if let Some(delta) = delta(&points, map, metres, seed) {
      let mut trunk = delta.trunk;
      set_curvature(&mut trunk, metres);
      let mut reaches = vec![Reach {
        points: trunk,
        ..Reach::default()
      }];
      reaches.extend(delta.arms.into_iter().map(|points| Reach {
        points,
        ..Reach::default()
      }));
      return Shaped {
        reaches,
        falls,
        oxbows,
        loops,
        lines,
        fan: delta.fan,
      };
    }
  }

  let mut fan = Vec::new();

  if raw.mouth == Mouth::Sea && estuary(&mut points, map.metadata.sea_level_metres, metres) {
    fan = mouth_bars(&points, map, metres);
  }

  set_curvature(&mut points, metres);
  Shaped {
    // Painted rivers keep the path their author drew.
    reaches: braid(
      points,
      map,
      metres,
      options.braiding.clamp(0.0, 1.0) * f32::from(u8::from(!raw.painted)),
      seed,
    ),
    falls,
    oxbows,
    loops,
    lines,
    fan,
  }
}

/// Split the braided stretches of a stream into a belt and its threads.
/// A point braids by `braiding x smoothstep(S_b, 2 S_b, S)`, with `S_b`
/// the braiding threshold, where the channel is at least a sample wide,
/// the valley floor at least 3 w and the banks not rock: steep, wide and
/// unconfined (Leopold and Wolman, 1957; Parker, 1976). Over each run
/// braiding more than 0.25 for at least 10 w, the stream's own reach
/// becomes a belt `min(3 w, floor)` wide, carved flat 0.1 d below the
/// water (the threads' level) and never drawn, and 2 to 4 threads share its discharge, each
/// wandering across the belt along a seeded noise 3 to 6 w long, so they
/// merge and split again where their paths meet.
fn braid(
  points: Vec<ChannelPoint>,
  map: &HeightMap,
  metres: f32,
  braiding: f32,
  seed: u64,
) -> Vec<Reach> {
  let n = points.len();
  let main = |points: &[ChannelPoint]| Reach {
    points: points.to_vec(),
    ..Reach::default()
  };

  if braiding <= 0.0 || points.iter().all(|p| p.width < metres) {
    return vec![main(&points)];
  }

  let floors = floor_widths(&points, map, metres);
  let braided: Vec<f32> = points
    .iter()
    .zip(&floors)
    .map(|(p, floor)| {
      let threshold = braiding_threshold(p.discharge);
      let room = p.width >= metres && *floor >= 3.0 * p.width && !p.falling;
      braiding
        * smoothstep((p.slope - threshold) / threshold)
        * (1.0 - rock_banks(p))
        * f32::from(u8::from(room))
    })
    .collect();
  let s = arc_lengths(
    &points.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
    metres,
  );
  let mut reaches = Vec::new();
  let (mut from, mut i) = (0, 0);

  while i < n {
    let a = i;

    while i < n && braided[i] > 0.25 {
      i += 1;
    }

    if i == a {
      i += 1;
      continue;
    }

    let b = i - 1;

    if s[b] - s[a] < 10.0 * points[a].width {
      continue;
    }

    if a > from {
      reaches.push(main(&points[from..=a]));
    }

    let mean = braided[a..=b].iter().sum::<f32>() / (b - a + 1) as f32;
    let count = 2 + (2.0 * mean).round() as usize;

    for thread in 0..count {
      let thread_seed = hash_u64(seed ^ ((a as u64) << 8) ^ thread as u64);
      let length = 3.0 + 3.0 * (thread_seed >> 40) as f32 / (1u64 << 24) as f32;
      let mut points: Vec<ChannelPoint> = (a..=b)
        .map(|k| {
          let p = points[k];
          let (before, after) = (points[k.saturating_sub(1)], points[(k + 1).min(n - 1)]);
          let along = length2(after.x - before.x, after.y - before.y).max(1e-6);
          // Threads leave and rejoin the channel over 2 w at the ends.
          let taper = smoothstep((s[k] - s[a]).min(s[b] - s[k]) / (2.0 * p.width));
          let offset = 0.5 * (3.0 * p.width).min(floors[k]) * taper / metres
            * value_noise(thread_seed, s[k] / (length * p.width), 0.0);
          let q = p.discharge / count as f32;
          let d = channel_depth(q);
          let level = p.level - 0.1 * p.depth;
          ChannelPoint {
            x: p.x - (after.y - before.y) / along * offset,
            y: p.y + (after.x - before.x) / along * offset,
            level,
            bed: level - d,
            width: channel_width(q, p.width / channel_width(p.discharge, 1.0)),
            depth: d,
            discharge: q,
            speed: manning_speed(d, p.slope),
            ..p
          }
        })
        .collect();
      set_curvature(&mut points, metres);
      reaches.push(Reach {
        points,
        kind: ReachKind::Thread,
      });
    }

    reaches.push(Reach {
      points: (a..=b)
        .map(|k| {
          let p = points[k];
          // Its water is the threads': the floor is carved flat to it.
          let level = p.level - 0.1 * p.depth;
          ChannelPoint {
            width: (3.0 * p.width).min(floors[k]),
            level,
            bed: level,
            depth: 0.0,
            ..p
          }
        })
        .collect(),
      kind: ReachKind::Belt,
    });
    from = b;
  }

  if n - from >= 2 {
    reaches.push(main(&points[from..]));
  }

  reaches
}

/// The floodplain a meandering river built. `lines` holds its final
/// centreline, then the snapshots of its migration. Inside their envelope,
/// widened by w each side, ground between `level + 0.25 d + 0.3 m` and
/// the valley walls' foot, `level + max(1.5 d, 1 m)`, is lowered to the
/// former, blending back to the ground over 2 w beyond; the walls are
/// never touched. Along the snapshots, where they lie more than 1.5 w
/// from the river, swales `min(0.3 m, 0.15 d)` deep and half a width wide
/// mark the bends' old places: the scroll bars on the inside of loops.
fn carve_belt(
  map: &mut HeightMap,
  lines: &[Vec<ChannelPoint>],
  metres: f32,
  record: &mut CarveRecord,
  near: &mut Near,
) {
  let (width, height) = (map.metadata.width as i32, map.metadata.height as i32);
  let sea = map.metadata.sea_level_metres;

  for (line, points) in lines.iter().enumerate() {
    for pair in points.windows(2) {
      let (a, b) = (pair[0], pair[1]);
      let w = a.width / metres;
      let floor = a.level + 0.25 * a.depth + 0.3;
      let top = a.level + (1.5 * a.depth).max(1.0);
      let swale = (0.25 * w).max(0.5);
      let (dx, dy) = (b.x - a.x, b.y - a.y);
      let length = (dx * dx + dy * dy).max(1e-6);
      let reach = (3.0 * w).ceil() as i32 + 1;

      for y in (a.y.min(b.y) as i32 - reach).max(0)..=(a.y.max(b.y) as i32 + reach).min(height - 1)
      {
        for x in (a.x.min(b.x) as i32 - reach).max(0)..=(a.x.max(b.x) as i32 + reach).min(width - 1)
        {
          let (px, py) = (x as f32 - a.x, y as f32 - a.y);
          let t = ((px * dx + py * dy) / length).clamp(0.0, 1.0);
          let distance = length2(px - dx * t, py - dy * t);
          let index = (y * width + x) as usize;
          let ground = map.heights[index];

          if distance > 3.0 * w || map.no_data[index] || ground <= sea || ground > top {
            continue;
          }

          if line == 0 && distance <= 1.5 * w && !near.flags[index] {
            near.flags[index] = true;
            near.set.push(index);
          }

          let target = if line > 0 && !near.flags[index] && distance <= swale {
            floor - (0.3f32).min(0.15 * a.depth)
          } else {
            floor + (ground - floor) * smoothstep((distance - w) / (2.0 * w))
          };

          if ground > target {
            record.lower(map, index, target);
          }
        }
      }
    }
  }

  for index in near.set.drain(..) {
    near.flags[index] = false;
  }
}

/// Samples within 1.5 w of a river, where its belt cuts no swale: a
/// flag a sample, kept from stream to stream with the ones set listed,
/// so each belt clears only what it set instead of allocating the map.
struct Near {
  flags: Vec<bool>,
  set: Vec<usize>,
}

/// The oxbow lake a cut-off loop leaves: still water 0.2 d below the
/// level where the loop left the channel. Older loops have silted up:
/// both ends are trimmed by up to 15 % of the loop, and the hollow is up
/// to 60 % shallower.
fn oxbow(cut: &CutLoop) -> Option<Oxbow> {
  let n = cut.points.len();
  let trim = (0.15 * cut.age * n as f32) as usize;
  let neck = &cut.points[0];
  let points: Vec<[f32; 2]> = cut.points[trim..n - trim]
    .iter()
    .map(|p| [p.x, p.y])
    .collect();

  (points.len() >= 3).then_some(Oxbow {
    points,
    width: neck.width,
    surface: neck.level - 0.2 * neck.depth,
    depth: 0.6 * neck.depth * (1.0 - 0.6 * cut.age),
    celsius: neck.celsius,
  })
}

fn set_curvature(points: &mut [ChannelPoint], metres: f32) {
  let n = points.len();

  for i in 1..n.saturating_sub(1) {
    let (a, b, c) = (points[i - 1], points[i], points[i + 1]);
    let u = [b.x - a.x, b.y - a.y];
    let v = [c.x - b.x, c.y - b.y];
    let lu = length2(u[0], u[1]);
    let lv = length2(v[0], v[1]);

    if lu < 1e-5 || lv < 1e-5 {
      continue;
    }

    // The sine of the turn: the same as the angle for the gentle turns of
    // a smoothed centreline.
    let turn = (u[0] * v[1] - u[1] * v[0]) / (lu * lv);
    let length = (lu + lv) * 0.5 * metres;
    points[i].curvature = (turn / length * b.width * 2.0).clamp(-1.0, 1.0);
  }
}

/// A river mouth split into distributaries.
struct Delta {
  trunk: Vec<ChannelPoint>,
  arms: Vec<Vec<ChannelPoint>>,
  /// Samples raised into the fan, with their new heights.
  fan: Vec<(usize, f32)>,
}

/// Split a large river meeting the sea on flat ground into distributaries
/// that divide at mouth bars (Edmonds and Slingerland, 2007). From the
/// delta head, 8 w before the mouth, two arms leave at 15 to 35 degrees
/// either side, sharing the discharge 50/50 ± 20 %. Each grows along the
/// seaward fall of the ground, turning by at most 4 degrees per width, and
/// after 8 to 14 of its widths splits the same way, up to three
/// generations (8 arms). An arm stops where the sea is deeper than 1.5 d
/// under it, at the map edge, or where it would come within its width of
/// another arm. A low fan of silt is raised within 3 w of the arms.
fn delta(points: &[ChannelPoint], map: &HeightMap, metres: f32, seed: u64) -> Option<Delta> {
  let n = points.len();
  let last = points[n.checked_sub(2)?];

  if last.width <= DELTA_WIDTH {
    return None;
  }

  let reach = 8.0 * last.width;
  let s = arc_lengths(
    &points.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
    metres,
  );
  let total = s[n - 1];
  let split = s.iter().position(|value| *value >= total - reach)?;

  if split < 2 {
    return None;
  }

  let head = points[split];
  let slope = (head.level - points[n - 1].level) / (total - s[split]).max(1.0);

  if slope >= DELTA_SLOPE {
    return None;
  }

  let sea = map.metadata.sea_level_metres;
  let (width, height) = (map.metadata.width as f32, map.metadata.height as f32);
  let scale = head.width / channel_width(head.discharge, 1.0);
  let random = |k: u64| {
    (hash_u64(seed ^ k.wrapping_mul(0x9e37_79b9_7f4a_7c15)) >> 40) as f32 / (1u32 << 24) as f32
  };
  let rotate = |d: [f32; 2], a: f32| {
    let (sin, cos) = a.sin_cos();
    [d[0] * cos - d[1] * sin, d[0] * sin + d[1] * cos]
  };
  let before = points[split - 2];
  let length = length2(head.x - before.x, head.y - before.y).max(1e-4);
  let along = [(head.x - before.x) / length, (head.y - before.y) / length];
  // Arms waiting to grow: first point, heading, discharge, generation and
  // distance from the delta head in metres.
  let mut waiting = vec![(head, along, head.discharge, 0u32, 0.0f32)];
  let mut arms: Vec<Vec<ChannelPoint>> = Vec::new();
  let mut next = 0;

  while next < waiting.len() {
    let (start, heading, q, generation, from_head) = waiting[next];
    let id = next as u64 * 16;
    next += 1;

    let w = channel_width(q, scale);
    let d = channel_depth(q);
    let step = crate::terrain::centreline::spacing(w, metres);
    let split_after = (8.0 + 6.0 * random(id + 1)) * w;
    let mut arm = vec![ChannelPoint {
      width: w,
      depth: d,
      discharge: q,
      ..start
    }];
    let mut direction = heading;
    let mut travelled = 0.0;
    // The delta head (generation 0) only splits; arms of the third
    // generation grow but never split.
    let mut splits = false;

    if generation > 0 {
      // An arm ends 40 widths from its start, a step at a time, so it
      // never takes more steps than this; the bound stops a step that is
      // not a number.
      let most = (40.0 * head.width / step).ceil() as usize + 2;

      for _ in 0..most.min(1 << 20) {
        let p = arm[arm.len() - 1];
        // Along the seaward fall of the ground, smoothed over 2 samples,
        // with a seeded wander; at most 4 degrees per width.
        let fall = [
          height_at(map, p.x - 2.0, p.y) - height_at(map, p.x + 2.0, p.y),
          height_at(map, p.x, p.y - 2.0) - height_at(map, p.x, p.y + 2.0),
        ];
        let most = (4.0f32).to_radians() * step / w;
        let toward = if length2(fall[0], fall[1]) > 1e-4 {
          let cross = direction[0] * fall[1] - direction[1] * fall[0];
          let dot = direction[0] * fall[0] + direction[1] * fall[1];
          0.5 * cross.atan2(dot)
        } else {
          0.0
        };
        let wander = (random(id + 2 + arm.len() as u64 * 7) - 0.5) * most;
        direction = rotate(direction, (toward + wander).clamp(-most, most));
        let (x, y) = (
          p.x + direction[0] * step / metres,
          p.y + direction[1] * step / metres,
        );
        travelled += step;
        let edge = x < 1.0 || y < 1.0 || x > width - 2.0 || y > height - 2.0;
        // Not across another arm, away from the bar this one leaves.
        let clear_of_bar =
          |o: &ChannelPoint| length2(o.x - start.x, o.y - start.y) * metres > 2.0 * w;
        let crossing = travelled > 2.0 * w
          && arms
            .iter()
            .flatten()
            .any(|o| clear_of_bar(o) && length2(o.x - x, o.y - y) * metres < w);

        if edge || crossing || travelled > 40.0 * head.width {
          break;
        }

        let level = sea + (head.level - sea) * (1.0 - (from_head + travelled) / reach).max(0.0);
        arm.push(ChannelPoint {
          x,
          y,
          level,
          bed: level - d,
          speed: manning_speed(d, head.slope),
          ..arm[0]
        });

        if height_at(map, x, y) < sea - 1.5 * d {
          break;
        }

        if generation < 3 && travelled >= split_after {
          splits = true;
          break;
        }
      }
    } else {
      splits = true;
    }

    if splits {
      let bar = arm[arm.len() - 1];
      let share = 0.3 + 0.4 * random(id + 3);

      for (side, part) in [(1.0, share), (-1.0, 1.0 - share)] {
        let angle = side * (15.0 + 20.0 * random(id + 4 + side as u64)).to_radians();
        waiting.push((
          bar,
          rotate(direction, angle),
          q * part,
          generation + 1,
          from_head + travelled,
        ));
      }
    }

    if arm.len() > 1 {
      let mut pins = vec![false; arm.len()];
      pins[arm.len() - 1] = true;
      let mut smooth = crate::terrain::centreline::resample(
        &arm,
        &arm.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
        &pins,
        metres,
      )
      .points;
      set_curvature(&mut smooth, metres);
      arms.push(smooth);
    }
  }

  // The fan: silt raised to half a metre above the sea near the arms,
  // dipping under it 3 w out.
  let (map_width, map_height) = (map.metadata.width as i32, map.metadata.height as i32);
  let mut raised: Vec<(usize, f32)> = Vec::new();

  for p in arms.iter().flatten() {
    let radius = 3.0 * p.width / metres;
    let r = radius.ceil() as i32;

    for y in (p.y as i32 - r).max(0)..=(p.y as i32 + r).min(map_height - 1) {
      for x in (p.x as i32 - r).max(0)..=(p.x as i32 + r).min(map_width - 1) {
        let distance = length2(x as f32 - p.x, y as f32 - p.y) / radius;
        let index = (y * map_width + x) as usize;
        let target = sea + 0.5 - (distance - 0.6).max(0.0) * 2.5;

        if distance <= 1.0 && !map.no_data[index] && map.heights[index] < target {
          raised.push((index, target));
        }
      }
    }
  }

  Some(Delta {
    trunk: points[..=split].to_vec(),
    arms,
    fan: raised,
  })
}

/// Samples one reach touches, and the height each is cut to, gathered
/// before any is lowered. Reused for every reach, so it is never
/// reallocated per reach.
#[derive(Clone, Debug, Default)]
pub struct CarveScratch {
  /// Height per sample after the segments so far, or infinity where
  /// untouched.
  height: Vec<f32>,
  /// Whether a sample lies inside the channel mask.
  inside: Vec<bool>,
  /// Samples with a height, in the order first touched.
  touched: Vec<usize>,
  /// The highest ground in each block of [`BLOCK`] x [`BLOCK`] samples,
  /// which bounds how far a V wall can reach.
  block_max: Vec<f32>,
  blocks_x: usize,
}

/// Edge of a [`CarveScratch::block_max`] block, in samples.
const BLOCK: usize = 4;

impl CarveScratch {
  /// Scratch for `map`, with its block maxima.
  pub fn new(map: &HeightMap) -> Self {
    let width = map.metadata.width as usize;
    let height = map.metadata.height as usize;
    let blocks_x = width.div_ceil(BLOCK);
    let mut block_max = vec![f32::NEG_INFINITY; blocks_x * height.div_ceil(BLOCK)];

    for (index, ground) in map.heights.iter().enumerate() {
      let block = (index / width / BLOCK) * blocks_x + index % width / BLOCK;
      block_max[block] = block_max[block].max(*ground);
    }

    Self {
      height: vec![f32::INFINITY; map.heights.len()],
      inside: vec![false; map.heights.len()],
      touched: Vec::new(),
      block_max,
      blocks_x,
    }
  }

  /// Note a sample raised above the ground it had (a delta fan).
  fn raised(&mut self, index: usize, width: usize, height: f32) {
    let block = (index / width / BLOCK) * self.blocks_x + index % width / BLOCK;
    self.block_max[block] = self.block_max[block].max(height);
  }

  /// The highest ground over samples `x0..=x1`, `y0..=y1`.
  fn highest(&self, x0: i32, x1: i32, y0: i32, y1: i32) -> f32 {
    let mut highest = f32::NEG_INFINITY;

    for by in y0 as usize / BLOCK..=y1 as usize / BLOCK {
      for bx in x0 as usize / BLOCK..=x1 as usize / BLOCK {
        highest = highest.max(self.block_max[by * self.blocks_x + bx]);
      }
    }

    highest
  }
}

/// The lake at a heightmap sample, from the flow grid.
fn lake_at(hydrology: &Hydrology, x: i32, y: i32) -> u32 {
  let stride = hydrology.stride.max(1) as i32;
  let gx = ((x + stride / 2) / stride).min(hydrology.width as i32 - 1);
  let gy = ((y + stride / 2) / stride).min(hydrology.height as i32 - 1);
  hydrology
    .lake
    .get((gy * hydrology.width as i32 + gx) as usize)
    .copied()
    .unwrap_or(NO_LAKE)
}

/// Cut one reach into the map in two passes. The first walks each
/// segment's box, reading each sample's ground once and cutting a scratch
/// copy of it segment by segment; the second checks the glacier, lake and
/// no-data rules and lowers each sample in the map once.
/// Consecutive segments are about a sample apart and their boxes overlap,
/// so the first pass does only the arithmetic that must be repeated.
#[allow(clippy::too_many_arguments)]
fn carve_reach(
  map: &mut HeightMap,
  hydrology: &Hydrology,
  surface: &[SurfaceSample],
  reach: &Reach,
  metres: f32,
  record: &mut CarveRecord,
  mask: &mut [bool],
  scratch: &mut CarveScratch,
) {
  let width = map.metadata.width as i32;
  let height = map.metadata.height as i32;
  let sea = map.metadata.sea_level_metres;
  let steepness = |slope: f32| smoothstep((slope - FLAT_SLOPE) / (V_SLOPE - FLAT_SLOPE));

  for pair in reach.points.windows(2) {
    let (a, b) = (pair[0], pair[1]);
    let w = a.width.max(b.width);
    // At least three quarters of a sample, so a diagonal reach also cuts
    // the two samples beside it and the water never breaks up between
    // samples.
    let r = (0.5 * w).max(0.75 * metres);
    let wall = rock_banks(&a).max(rock_banks(&b));
    let steep = steepness(a.slope.max(b.slope)).max(wall);
    // V walls may climb several samples up a steep valley side; on gentle
    // ground the floodplain meets the ground 4 w beyond the bank, and
    // nothing further out is cut.
    // A big river in a V-shaped valley reads as a canal, so on gentle
    // ground it flattens a floor 3 w wide beside each bank, blending back
    // to the ground over a further 6 w.
    let valley = w >= VALLEY_FLOOR_WIDTH && a.slope.max(b.slope) < VALLEY_FLOOR_SLOPE;
    let plain_metres = if valley { 9.0 * w } else { 4.0 * w };
    let reach_metres = if steep > 0.0 {
      r + (4.0 * w * (1.0 - steep)).max(3.0 * metres)
    } else {
      r + plain_metres
    };
    let mask_metres = (0.65 * w).max(0.5 * metres);
    let low_level = a.level.min(b.level);
    // A V wall reaches only as far as the ground rises above the water,
    // so the highest ground near the segment bounds the cut.
    let reach_metres = if steep > 0.0 {
      let box_samples = (reach_metres / metres).ceil() as i32 + 1;
      let highest = scratch.highest(
        (a.x.min(b.x).floor() as i32 - box_samples).max(0),
        (a.x.max(b.x).ceil() as i32 + box_samples).min(width - 1),
        (a.y.min(b.y).floor() as i32 - box_samples).max(0),
        (a.y.max(b.y).ceil() as i32 + box_samples).min(height - 1),
      );
      reach_metres.min(r + (4.0 * w).max(highest - low_level))
    } else {
      reach_metres
    };
    let reach_samples = (reach_metres / metres).ceil() as i32 + 1;
    let min_x = (a.x.min(b.x).floor() as i32 - reach_samples).max(0);
    let max_x = (a.x.max(b.x).ceil() as i32 + reach_samples).min(width - 1);
    let min_y = (a.y.min(b.y).floor() as i32 - reach_samples).max(0);
    let max_y = (a.y.max(b.y).ceil() as i32 + reach_samples).min(height - 1);
    let segment = [b.x - a.x, b.y - a.y];
    let inv_length_sq = 1.0 / (segment[0] * segment[0] + segment[1] * segment[1]).max(1e-8);
    let radius = reach_metres / metres;
    let reach_sq = radius * radius;
    let mask_sq = (mask_metres / metres) * (mask_metres / metres);
    let shape = ChannelShape::new(r, w, steep, wall, valley);
    let r_samples = r / metres;
    let plain_sq = ((r + plain_metres) / metres).powi(2);

    for y in min_y..=max_y {
      let py = y as f32 - a.y;
      let Some((from, to)) = capsule_row(segment, radius, py) else {
        continue;
      };
      let first = ((a.x + from).ceil() as i32).max(min_x);
      let last = ((a.x + to).floor() as i32).min(max_x);
      let row = (y * width) as usize;

      for x in first..=last {
        let px = x as f32 - a.x;
        let t = ((px * segment[0] + py * segment[1]) * inv_length_sq).clamp(0.0, 1.0);
        let (dx, dy) = (px - segment[0] * t, py - segment[1] * t);
        let distance_sq = dx * dx + dy * dy;

        if distance_sq > reach_sq {
          continue;
        }

        let index = row + x as usize;

        // Beyond the floodplain the bed's profile is the ground itself, so
        // only a V wall cuts, and only into ground more than `d - r` above
        // the water. Carving only lowers, so the original ground bounds it.
        if distance_sq >= plain_sq {
          let rise = map.heights[index] - low_level;

          if steep <= 0.0 || rise <= 0.0 || distance_sq >= (r_samples + rise / metres).powi(2) {
            continue;
          }
        }

        if scratch.height[index] == f32::INFINITY {
          scratch.height[index] = map.heights[index];
          scratch.touched.push(index);
        }

        // The ground as the segments before this one left it. Once it is
        // cut to the sea it is left there, as a coast.
        let ground = scratch.height[index];

        if ground <= sea {
          continue;
        }

        scratch.inside[index] |= distance_sq <= mask_sq;
        let level = a.level + (b.level - a.level) * t;
        let depth = a.depth + (b.depth - a.depth) * t;
        // Positive on the outer side of a bend: a left turn's outer bank is
        // on its right.
        let curvature = a.curvature + (b.curvature - a.curvature) * t;
        let right = segment[0] * dy - segment[1] * dx < 0.0;
        let bend = if right == (curvature > 0.0) {
          curvature.abs()
        } else {
          -curvature.abs()
        };
        let target = shape.target(ground, level, depth, distance_sq.sqrt() * metres, bend);
        scratch.height[index] = ground.min(target);
      }
    }
  }

  for index in scratch.touched.drain(..) {
    let target = std::mem::replace(&mut scratch.height[index], f32::INFINITY);
    let inside = std::mem::take(&mut scratch.inside[index]);
    let (x, y) = (
      (index % width as usize) as i32,
      (index / width as usize) as i32,
    );

    if map.no_data[index]
      || map.heights[index] <= sea
      || surface.get(index).is_some_and(|s| s.is_glacier())
      || lake_at(hydrology, x, y) != NO_LAKE
    {
      continue;
    }

    record.lower(map, index, target);

    if inside {
      mask[index] = true;
    }
  }
}

/// The span of x offsets, from a segment's start, where a row `py` below
/// it comes within `radius` of the segment `(0, 0)` to `segment`: the row
/// through a capsule, which is convex, so one interval. `None` where the
/// row misses it.
fn capsule_row(segment: [f32; 2], radius: f32, py: f32) -> Option<(f32, f32)> {
  let mut span: Option<(f32, f32)> = None;
  let mut include = |from: f32, to: f32| {
    if from <= to {
      span = Some(span.map_or((from, to), |(a, b)| (a.min(from), b.max(to))));
    }
  };

  // The discs at either end.
  for (cx, cy) in [(0.0, 0.0), (segment[0], segment[1])] {
    let rise = py - cy;

    if rise.abs() <= radius {
      let half = (radius * radius - rise * rise).sqrt();
      include(cx - half, cx + half);
    }
  }

  // The band beside the segment: its projection on the segment within
  // the segment, and within `radius` of the line. Both are linear in x.
  let (sx, sy) = (segment[0], segment[1]);
  let length = length2(sx, sy);

  if length > 1e-6 {
    // Solve `lo <= k x + c <= hi` for x, as an interval.
    let solve = |k: f32, c: f32, lo: f32, hi: f32| -> Option<(f32, f32)> {
      if k.abs() < 1e-9 {
        return (lo <= c && c <= hi).then_some((f32::NEG_INFINITY, f32::INFINITY));
      }

      let (p, q) = ((lo - c) / k, (hi - c) / k);
      Some((p.min(q), p.max(q)))
    };
    let along = solve(sx, sy * py, 0.0, length * length);
    let across = solve(-sy, sx * py, -radius * length, radius * length);

    if let (Some(along), Some(across)) = (along, across) {
      include(along.0.max(across.0), along.1.min(across.1));
    }
  }

  span
}

/// The cross-section of a channel segment: a flat bed with a floodplain
/// bank on gentle ground, blended by `steep` into a V in steep ground,
/// with its divisions done once per segment.
struct ChannelShape {
  /// Half-width of the cut, in metres.
  r: f32,
  inv_r: f32,
  /// Where the flat bed ends.
  inner: f32,
  inv_band: f32,
  /// One over the floodplain's reach, 4 w.
  inv_plain: f32,
  steep: f32,
  /// The wall slope from `r` to `r + w`: 1 (45 degrees), up to 6 where
  /// the banks are rock (see [`rock_banks`]).
  wall_slope: f32,
  w: f32,
  /// Where a big river's valley floor ends, 3 w beyond the bank, and one
  /// over the 6 w it takes to meet the ground; `None` for other rivers.
  valley: Option<(f32, f32)>,
}

impl ChannelShape {
  fn new(r: f32, w: f32, steep: f32, wall: f32, valley: bool) -> Self {
    Self {
      r,
      inv_r: 1.0 / r,
      inner: 0.7 * r,
      inv_band: 1.0 / (0.3 * r),
      inv_plain: 1.0 / (4.0 * w),
      steep,
      wall_slope: 1.0 + 5.0 * wall,
      w,
      valley: valley.then(|| (r + 3.0 * w, 1.0 / (6.0 * w))),
    }
  }

  /// Target ground height at `distance` metres from the centreline.
  /// Written without branches: inside the bank the floodplain term is
  /// exactly `bank`, outside it the bank term is, and the two pieces of
  /// the V meet at `level`, so each piece is simply clamped.
  ///
  /// On a bend, `bend` is the channel's curvature (0 to 1) on its outer
  /// side and minus that on its inner side. The thalweg shifts `0.3 a r`
  /// towards the outer bank, a cut bank whose wall is `2 a` steeper; the
  /// inner bank is a point bar, a straight slope from the thalweg up to
  /// 0.1 d below the water at the bank edge, carved less, never raised.
  /// The shift fades out between the bank and `2 r`, so the floodplain
  /// beyond is as before. Straight reaches keep the symmetric section
  /// exactly.
  fn target(&self, ground: f32, level: f32, depth: f32, distance: f32, bend: f32) -> f32 {
    let r = self.r;
    let a = bend.abs();
    let outer = f32::from(u8::from(bend > 0.0));
    let shift = 0.3 * a * r * (2.0 - distance * self.inv_r).clamp(0.0, 1.0);
    let distance = if bend > 0.0 {
      (distance - shift).abs()
    } else {
      distance + shift
    };
    let bed = level - depth;
    let bank = level + 0.25 * depth;
    let rise = ((distance - self.inner) * self.inv_band).clamp(0.0, 1.0);
    let plain = smoothstep((distance - r) * self.inv_plain);
    let trapezoid = bed + (bank - bed) * rise + (ground - bank) * plain;
    let beyond = (distance - r).max(0.0);
    let v_shape = bed
      + depth * distance.min(r) * self.inv_r
      + beyond
      + (self.wall_slope + 2.0 * a * outer - 1.0) * beyond.min(self.w);
    let mut target = trapezoid + (v_shape - trapezoid) * self.steep;

    if bend < 0.0 && distance < r {
      let bar = bed + 0.9 * depth * distance * self.inv_r;
      target += (target.max(bar) - target) * a;
    }

    match self.valley {
      // At most half a metre above the bank, then back to the ground.
      Some((floor_end, inv_blend)) => {
        let floor = bank + 0.5;
        let blend = smoothstep((distance - floor_end) * inv_blend);
        target.min(floor + (ground - floor) * blend)
      }
      None => target,
    }
  }
}

/// The carve before [`carve_reach`] visited each sample once: kept as a
/// reference for its tests.
#[cfg(test)]
fn carve_reach_reference(
  map: &mut HeightMap,
  hydrology: &Hydrology,
  surface: &[SurfaceSample],
  reach: &Reach,
  metres: f32,
  record: &mut CarveRecord,
  mask: &mut [bool],
) {
  let width = map.metadata.width as i32;
  let height = map.metadata.height as i32;
  let sea = map.metadata.sea_level_metres;
  let points = &reach.points;
  let lake_at = |x: i32, y: i32| {
    let stride = hydrology.stride.max(1) as i32;
    let gx = ((x + stride / 2) / stride).min(hydrology.width as i32 - 1);
    let gy = ((y + stride / 2) / stride).min(hydrology.height as i32 - 1);
    hydrology
      .lake
      .get((gy * hydrology.width as i32 + gx) as usize)
      .copied()
      .unwrap_or(NO_LAKE)
  };

  for pair in points.windows(2) {
    let (a, b) = (pair[0], pair[1]);
    let w = a.width.max(b.width);
    // At least three quarters of a sample, so a diagonal reach also cuts
    // the two samples beside it and the water never breaks up between
    // samples.
    let r = (0.5 * w).max(0.75 * metres);
    let wall = rock_banks(&a).max(rock_banks(&b));
    let steep = smoothstep((a.slope.max(b.slope) - FLAT_SLOPE) / (V_SLOPE - FLAT_SLOPE)).max(wall);
    let reach_metres = r + (4.0 * w * (1.0 - steep)).max(3.0 * metres);
    let reach_samples = (reach_metres / metres).ceil() as i32 + 1;
    let min_x = (a.x.min(b.x).floor() as i32 - reach_samples).max(0);
    let max_x = (a.x.max(b.x).ceil() as i32 + reach_samples).min(width - 1);
    let min_y = (a.y.min(b.y).floor() as i32 - reach_samples).max(0);
    let max_y = (a.y.max(b.y).ceil() as i32 + reach_samples).min(height - 1);
    let segment = [b.x - a.x, b.y - a.y];
    let length_sq = (segment[0] * segment[0] + segment[1] * segment[1]).max(1e-8);

    for y in min_y..=max_y {
      for x in min_x..=max_x {
        let px = x as f32 - a.x;
        let py = y as f32 - a.y;
        let t = ((px * segment[0] + py * segment[1]) / length_sq).clamp(0.0, 1.0);
        let (ox, oy) = (px - segment[0] * t, py - segment[1] * t);
        let distance = length2(ox, oy) * metres;

        if distance > reach_metres {
          continue;
        }

        let index = (y * width + x) as usize;
        let ground = map.heights[index];

        if map.no_data[index]
          || ground <= sea
          || surface.get(index).is_some_and(|s| s.is_glacier())
          || lake_at(x, y) != NO_LAKE
        {
          continue;
        }

        let level = a.level + (b.level - a.level) * t;
        let depth = a.depth + (b.depth - a.depth) * t;
        let bed = level - depth;
        // On a bend, the outer side (right of a left turn) is cut deeper and
        // steeper, and the inner side is a point bar.
        let curvature = a.curvature + (b.curvature - a.curvature) * t;
        let bend = curvature.abs();
        let outer = (segment[0] * oy - segment[1] * ox < 0.0) == (curvature > 0.0);
        let shift = if distance <= r {
          0.3 * bend * r
        } else if distance < 2.0 * r {
          0.3 * bend * (2.0 * r - distance)
        } else {
          0.0
        };
        let distance = if outer {
          (distance - shift).abs()
        } else {
          distance + shift
        };
        let steeper = if outer { 2.0 * bend } else { 0.0 };
        let v_shape = if distance <= r {
          bed + depth * distance / r
        } else {
          level + (distance - r) + (5.0 * wall + steeper) * (distance - r).min(w)
        };
        let bank = level + 0.25 * depth;
        let trapezoid = if distance <= 0.7 * r {
          bed
        } else if distance <= r {
          bed + (bank - bed) * (distance - 0.7 * r) / (0.3 * r)
        } else {
          bank + (ground - bank) * smoothstep((distance - r) / (4.0 * w))
        };
        let mut target = trapezoid + (v_shape - trapezoid) * steep;

        if !outer && bend > 0.0 && distance < r {
          let bar = bed + 0.9 * depth * distance / r;
          target += (target.max(bar) - target) * bend;
        }

        record.lower(map, index, target);

        if distance <= (0.65 * w).max(0.5 * metres) {
          mask[index] = true;
        }
      }
    }
  }
}

/// Cut a plunge pool: a bowl of the fall's pool radius and depth below the
/// water level. Ground is only ever lowered, so on a steep slope the bowl
/// is cut into the hillside and never builds a terrace below it; the
/// shader shows water only where the bowl holds it.
fn carve_pool(
  map: &mut HeightMap,
  fall: &Fall,
  metres: f32,
  record: &mut CarveRecord,
  mask: &mut [bool],
) {
  let width = map.metadata.width as i32;
  let height = map.metadata.height as i32;
  let radius = fall.pool_radius / metres;
  let r = radius.ceil() as i32 + 1;
  let (cx, cy) = (fall.foot[0], fall.foot[1]);

  for y in (cy as i32 - r).max(0)..=(cy as i32 + r).min(height - 1) {
    for x in (cx as i32 - r).max(0)..=(cx as i32 + r).min(width - 1) {
      let d = length2(x as f32 - cx, y as f32 - cy) / radius.max(1e-4);
      let index = (y * width + x) as usize;

      if d >= 1.0 || map.no_data[index] {
        continue;
      }

      record.lower(
        map,
        index,
        fall.foot_level - fall.pool_depth * (1.0 - d * d),
      );
      mask[index] = true;
    }
  }
}

/// Carve an oxbow: a flat hollow its depth below its surface.
fn carve_oxbow(map: &mut HeightMap, oxbow: &Oxbow, metres: f32, record: &mut CarveRecord) {
  let width = map.metadata.width as i32;
  let height = map.metadata.height as i32;
  let depth = oxbow.depth;
  let radius = (oxbow.width * 0.5).max(0.75 * metres) / metres;
  let r = radius.ceil() as i32 + 1;

  for pair in oxbow.points.windows(2) {
    let (a, b) = (pair[0], pair[1]);
    let segment = [b[0] - a[0], b[1] - a[1]];
    let length_sq = (segment[0] * segment[0] + segment[1] * segment[1]).max(1e-8);

    for y in (a[1].min(b[1]) as i32 - r).max(0)..=(a[1].max(b[1]) as i32 + r).min(height - 1) {
      for x in (a[0].min(b[0]) as i32 - r).max(0)..=(a[0].max(b[0]) as i32 + r).min(width - 1) {
        let px = x as f32 - a[0];
        let py = y as f32 - a[1];
        let t = ((px * segment[0] + py * segment[1]) / length_sq).clamp(0.0, 1.0);

        if length2(px - segment[0] * t, py - segment[1] * t) > radius {
          continue;
        }

        let index = (y * width + x) as usize;

        if !map.no_data[index] && map.heights[index] > map.metadata.sea_level_metres {
          record.lower(map, index, oxbow.surface - depth);
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::terrain::heightmap::update_stats;
  use crate::terrain::hydrology::build_hydrology;
  use vista_types::TerrainMetadata;

  fn map_from(size: u32, metres: f32, height: impl Fn(f32, f32) -> f32) -> HeightMap {
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: metres,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 0.0, metadata);

    for y in 0..size {
      for x in 0..size {
        map.heights[(y * size + x) as usize] = height(x as f32, y as f32);
      }
    }

    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    map
  }

  fn condition(map: &mut HeightMap, options: &RiverOptions) -> (Channels, Vec<(usize, f32)>) {
    let hydrology = build_hydrology(map, &[], options, 9);
    let streams = raw_streams(&hydrology);
    let mut record = CarveRecord::new(map.heights.len());
    let context = ChannelContext {
      surface: &[],
      options,
      seed: 9,
    };
    let channels = condition_channels(map, &hydrology, streams, &context, &mut record);
    (channels, record.into_original())
  }

  /// Two valleys joining, draining north to the sea.
  fn branching_valleys() -> HeightMap {
    map_from(128, 30.0, |x, y| {
      let main = (x - 64.0).abs();
      let branch = (x - 64.0 - (y - 40.0).max(0.0) * 0.8).abs();
      y * 1.5 - 6.0 + main.min(branch) * 3.0
    })
  }

  #[test]
  fn steep_channels_are_rougher_and_slower() {
    // A lowland river keeps the plain roughness: 2 m deep on 0.1 %,
    // 1.587 x 0.0316 / 0.035 = 1.43 m/s.
    assert!((manning_speed(2.0, 0.001) - 1.433).abs() < 0.01);
    // A 4 % river 2 m deep runs at about 3 m/s, not at the 6 m/s ceiling.
    let steep = manning_speed(2.0, 0.04);
    assert!((2.8..3.6).contains(&steep), "{steep}");
    // A cascade 0.6 m deep on 27 % runs at under 2 m/s.
    assert!(manning_speed(0.6, 0.27) < 2.0);
    // Still faster the steeper it is.
    assert!(manning_speed(1.0, 0.08) > manning_speed(1.0, 0.04));
  }

  #[test]
  fn powerful_steep_reaches_get_rock_walls() {
    let point = |discharge: f32, slope: f32| ChannelPoint {
      x: 0.0,
      y: 0.0,
      level: 10.0,
      bed: 9.0,
      width: 5.0,
      depth: 1.0,
      discharge,
      slope,
      speed: 2.0,
      curvature: 0.0,
      celsius: 10.0,
      rapids: 0.0,
      falling: false,
      order: 1,
    };
    // 1000 x 9.81 x 10 x 0.05 / 5 = 981 W/m².
    assert!((stream_power(&point(10.0, 0.05)) - 981.0).abs() < 0.5);
    assert_eq!(rock_banks(&point(10.0, 0.05)), 1.0);
    // Powerful but gentle, or steep but weak: alluvial banks.
    assert_eq!(rock_banks(&point(100.0, 0.015)), 0.0);
    assert_eq!(rock_banks(&point(1.0, 0.05)), 0.0);
    // A rivulet on a mountainside has the power (882 W/m²) but not the
    // water to strip its banks to rock.
    assert!(stream_power(&point(0.9, 0.5)) > 600.0);
    assert_eq!(rock_banks(&point(0.9, 0.5)), 0.0);

    // Within r to r + w the wall climbs six times as steeply, so it cuts
    // less: the ground is only ever lowered.
    let soft = ChannelShape::new(2.5, 5.0, 1.0, 0.0, false);
    let rock = ChannelShape::new(2.5, 5.0, 1.0, 1.0, false);
    let at = |shape: &ChannelShape, distance: f32| shape.target(100.0, 10.0, 1.0, distance, 0.0);
    assert_eq!(at(&soft, 1.0), at(&rock, 1.0));
    assert!((at(&rock, 5.0) - at(&soft, 5.0) - 12.5).abs() < 1e-4);
    assert!((at(&rock, 10.0) - at(&soft, 10.0) - 25.0).abs() < 1e-4);
  }

  #[test]
  fn beds_never_rise_downstream() {
    let mut map = branching_valleys();
    let options = RiverOptions {
      min_catchment_km2: 0.3,
      ..RiverOptions::default()
    };
    let (channels, _) = condition(&mut map, &options);
    assert!(channels.reaches.len() >= 2);

    for reach in &channels.reaches {
      for pair in reach.points.windows(2) {
        assert!(
          pair[1].bed <= pair[0].bed + 1e-4,
          "bed rises {} -> {}",
          pair[0].bed,
          pair[1].bed
        );
        assert!(pair[1].level <= pair[0].level + 1e-4);
      }
    }
  }

  #[test]
  fn channels_widen_downstream_of_every_confluence() {
    let mut map = branching_valleys();
    let options = RiverOptions {
      min_catchment_km2: 0.3,
      meanders: 0.0,
      ..RiverOptions::default()
    };
    let hydrology = build_hydrology(&map, &[], &options, 9);
    let streams = raw_streams(&hydrology);
    let joins: Vec<[f32; 2]> = streams
      .iter()
      .filter(|s| s.mouth == Mouth::Join)
      .map(|s| *s.points.last().unwrap())
      .collect();
    assert!(!joins.is_empty());
    let mut record = CarveRecord::new(map.heights.len());
    let context = ChannelContext {
      surface: &[],
      options: &options,
      seed: 9,
    };
    let channels = condition_channels(&mut map, &hydrology, streams, &context, &mut record);

    for join in joins {
      let mut inputs = 0.0f32;
      let mut downstream = f32::INFINITY;

      for reach in &channels.reaches {
        for (i, p) in reach.points.iter().enumerate() {
          if p.x == join[0] && p.y == join[1] {
            if i > 0 {
              inputs = inputs.max(reach.points[i - 1].width);
            }

            if i + 1 < reach.points.len() {
              downstream = downstream.min(reach.points[i + 1].width);
            }
          }
        }
      }

      assert!(downstream >= inputs, "{downstream} < {inputs} at {join:?}");
    }
  }

  /// A synthetic floodplain: a straight valley along x at 12 m
  /// samples, falling `slope`, with a flat floor `floor` widths wide
  /// (w about 12 m for its 20 m³/s) between walls rising 4 m a sample.
  fn floodplain_map(slope: f32, floor: f32) -> HeightMap {
    let half = floor * 12.07 / 12.0 / 2.0;
    map_from(512, 12.0, |x, y| {
      20.0 - x * 12.0 * slope + 0.05 + ((y - 256.0).abs() - half).max(0.0) * 4.0
    })
  }

  /// The floodplain's river: 20 m³/s straight down the valley's middle.
  fn floodplain_river(slope: f32) -> RawStream {
    let n = 500;
    RawStream {
      points: (0..n).map(|i| [6.0 + i as f32, 256.0]).collect(),
      levels: (0..n)
        .map(|i| 20.0 - (6.0 + i as f32) * 12.0 * slope)
        .collect(),
      discharge: vec![20.0; n],
      orders: vec![2; n],
      min_width: 0.0,
      mouth: Mouth::Edge,
      painted: false,
    }
  }

  fn floodplain(meanders: f32, maturity: f32, slope: f32, floor: f32) -> Shaped {
    let options = RiverOptions {
      meanders,
      meander_maturity: maturity,
      ..RiverOptions::default()
    };
    let context = ChannelContext {
      surface: &[],
      options: &options,
      seed: 3,
    };
    let map = floodplain_map(slope, floor);
    shape_stream(&floodplain_river(slope), None, &map, 12.0, &context, 3)
  }

  fn sinuosity(points: &[ChannelPoint]) -> f32 {
    let length: f32 = points
      .windows(2)
      .map(|p| length2(p[1].x - p[0].x, p[1].y - p[0].y))
      .sum();
    let first = points[0];
    let last = points[points.len() - 1];
    length / length2(last.x - first.x, last.y - first.y)
  }

  #[test]
  fn floodplains_meander_more_as_they_mature_and_straight_options_do_not() {
    let shapes: Vec<Shaped> = [0.0, 0.25, 0.5, 0.75, 1.0]
      .map(|maturity| floodplain(1.0, maturity, 0.001, 40.0))
      .into();
    let sinuosities: Vec<f32> = shapes
      .iter()
      .map(|shaped| sinuosity(&shaped.reaches[0].points))
      .collect();

    for pair in sinuosities.windows(2) {
      assert!(pair[1] >= pair[0], "{sinuosities:?}");
    }

    assert!((1.4..=2.4).contains(&sinuosities[2]), "{sinuosities:?}");
    assert!(sinuosities[4] >= 1.6, "{sinuosities:?}");
    assert!(!shapes[4].oxbows.is_empty(), "no neck cut-off");
    let straight = floodplain(0.0, 1.0, 0.001, 40.0);
    let straight = sinuosity(&straight.reaches[0].points);
    assert!(straight <= 1.05, "sinuosity {straight}");

    let mature = &shapes[4].reaches[0].points;
    assert!(!crate::terrain::meander::crosses_itself(mature));
    // The ends stay where they were, so joins and the map edge hold.
    let (first, last) = (mature[0], mature[mature.len() - 1]);
    assert!((first.y - 256.0).abs() < 0.01 && (last.y - 256.0).abs() < 0.01);
    // A longer path over the same fall is gentler.
    let (mut drop, mut run) = (0.0, 0.0);

    for pair in mature.windows(2) {
      let along = length2(pair[1].x - pair[0].x, pair[1].y - pair[0].y) * 12.0;
      drop += 0.5 * (pair[0].slope + pair[1].slope) * along;
      run += along;
    }

    let expected = 0.001 / sinuosities[4];
    assert!(
      ((drop / run) / expected - 1.0).abs() <= 0.1,
      "slope {} for {expected}",
      drop / run
    );
    // Deterministic.
    let again = floodplain(1.0, 1.0, 0.001, 40.0);
    assert_eq!(again.reaches, shapes[4].reaches);
    assert_eq!(again.oxbows, shapes[4].oxbows);
  }

  #[test]
  fn steep_or_confined_valleys_barely_meander() {
    let steep = floodplain(1.0, 1.0, 0.06, 40.0);
    let steep = sinuosity(&steep.reaches[0].points);
    assert!(steep <= 1.15, "sinuosity {steep} on a 6 % valley");

    let confined = floodplain(1.0, 1.0, 0.001, 2.0);
    let points = &confined.reaches[0].points;
    let map = floodplain_map(0.001, 2.0);

    for p in points {
      let h = (1.5 * p.depth).max(1.0);
      assert!(
        height_at(&map, p.x, p.y) <= p.level + h + 1e-3,
        "on the valley wall at {}, {}",
        p.x,
        p.y
      );
    }

    assert!(sinuosity(points) <= 1.2, "sinuosity {}", sinuosity(points));
  }

  #[test]
  fn older_oxbows_are_shorter_and_shallower() {
    let points: Vec<ChannelPoint> = (0..40)
      .map(|i| {
        let angle = i as f32 / 39.0 * 5.0;
        ChannelPoint {
          x: 100.0 + 3.0 * angle.cos(),
          y: 100.0 + 3.0 * angle.sin(),
          level: 10.0,
          bed: 9.0,
          width: 12.0,
          depth: 1.2,
          discharge: 20.0,
          slope: 0.001,
          speed: 0.5,
          curvature: 0.0,
          celsius: 12.0,
          rapids: 0.0,
          falling: false,
          order: 3,
        }
      })
      .collect();
    let lake = |age: f32| {
      oxbow(&CutLoop {
        points: points.clone(),
        connection: [100.0, 97.0],
        age,
      })
      .unwrap()
    };
    let (young, old) = (lake(0.1), lake(0.9));
    assert!(old.points.len() < young.points.len());
    assert!(old.depth < young.depth);
    // Still water just below the level where the loop left the channel.
    assert!(young.surface < 10.0 && young.surface > 9.7);
  }

  #[test]
  fn tributaries_find_the_migrated_main_stem_and_oxbows_stay_off_it() {
    let mut map = floodplain_map(0.001, 40.0);
    // Down the north wall from the north-west, then across the floor.
    let path: Vec<[f32; 2]> = (0..=96)
      .map(|i| [180.0 + i as f32, 160.0 + i as f32])
      .collect();
    let tributary = RawStream {
      levels: path.iter().map(|p| height_at(&map, p[0], p[1])).collect(),
      discharge: vec![2.0; path.len()],
      orders: vec![1; path.len()],
      points: path,
      min_width: 0.0,
      mouth: Mouth::Join,
      painted: false,
    };
    let options = RiverOptions {
      meanders: 1.0,
      meander_maturity: 1.0,
      ..RiverOptions::default()
    };
    let hydrology = build_hydrology(&map, &[], &options, 9);
    let mut record = CarveRecord::new(map.heights.len());
    let context = ChannelContext {
      surface: &[],
      options: &options,
      seed: 3,
    };
    let streams = vec![floodplain_river(0.001), tributary];
    let channels = condition_channels(&mut map, &hydrology, streams, &context, &mut record);
    let main = &channels.reaches[0].points;
    let tributary = &channels.reaches[1].points;
    let end = tributary[tributary.len() - 1];
    assert!(
      main
        .iter()
        .any(|p| length2(p.x - end.x, p.y - end.y) < 1e-3),
      "the tributary ends off the main stem"
    );
    let joins = crate::terrain::river_metrics::junction_angles(&channels.reaches, 12.0);
    assert_eq!(joins.len(), 1);
    assert!(
      (25.0..=90.0).contains(&joins[0].angle),
      "a join at {} degrees",
      joins[0].angle
    );

    // It never crosses the main stem on its way there.
    let crosses = |a: &ChannelPoint, b: &ChannelPoint, c: &ChannelPoint, d: &ChannelPoint| {
      let side = |o: &ChannelPoint, p: &ChannelPoint, q: &ChannelPoint| {
        (p.x - o.x) * (q.y - o.y) - (p.y - o.y) * (q.x - o.x)
      };
      side(c, d, a) * side(c, d, b) < 0.0 && side(a, b, c) * side(a, b, d) < 0.0
    };

    for t in tributary[..tributary.len() - 2].windows(2) {
      for m in main.windows(2) {
        assert!(
          !crosses(&t[0], &t[1], &m[0], &m[1]),
          "the tributary crosses the main stem at {}, {}",
          t[0].x,
          t[0].y
        );
      }
    }

    // Oxbows hold still water beside the channel, not in it.
    assert!(!channels.oxbows.is_empty());
    let width = map.metadata.width;

    for oxbow in &channels.oxbows {
      let n = oxbow.points.len();

      for p in &oxbow.points[n / 3..2 * n / 3] {
        let sample = (p[1].round() as u32 * width + p[0].round() as u32) as usize;
        assert!(!channels.mask[sample], "an oxbow in the channel at {p:?}");
      }
    }
  }

  /// A valley draining north with a 20 m cliff across it at y = 64.
  fn cliff_valley() -> HeightMap {
    map_from(128, 2.0, |x, y| {
      let step = if y >= 64.0 { 20.0 } else { 0.0 };
      y * 0.1 - 0.5 + (x - 64.0).abs() * 0.5 + step
    })
  }

  /// Options for the cliff fixtures: a river of `discharge` m³/s entering
  /// at the top of the valley, as the valley alone drains only a trickle.
  fn cliff_options(discharge: f32) -> RiverOptions {
    RiverOptions {
      min_catchment_km2: 0.005,
      inflow: vista_types::RiverInflows::List(vec![vista_types::RiverInflow {
        position: [0.0, 120.0],
        discharge_cubic_metres_per_second: discharge,
      }]),
      ..RiverOptions::default()
    }
  }

  #[test]
  fn a_cliff_step_makes_one_waterfall_with_a_plunge_pool() {
    let mut map = cliff_valley();
    let options = cliff_options(4.0);
    let (channels, _) = condition(&mut map, &options);

    assert_eq!(channels.falls.len(), 1, "falls {:?}", channels.falls);
    let fall = channels.falls[0].clone();
    assert!(!fall.trickle && fall.steps.is_empty());
    assert!(
      (fall.height() - 20.0).abs() <= 2.0,
      "height {}",
      fall.height()
    );
    // At 4 m³/s the pool keeps its original size.
    assert!((fall.pool_radius - (0.3 * fall.height() + fall.width)).abs() < 1e-3);

    // The pool is cut to about its radius around the foot, and no further.
    // The outlet channel downstream (north) of the foot is left out.
    let metres = map.metadata.metres_per_sample;
    let mut deepest_reach = 0.0f32;

    for y in fall.foot[1].floor() as u32..128u32 {
      for x in 0..128u32 {
        let index = (y * 128 + x) as usize;
        let below_foot = fall.foot_level - map.heights[index];
        let distance = length2(x as f32 - fall.foot[0], y as f32 - fall.foot[1]) * metres;

        if below_foot > fall.pool_depth * 0.5 && distance < fall.pool_radius * 2.0 {
          deepest_reach = deepest_reach.max(distance);
        }
      }
    }

    assert!(
      deepest_reach > fall.pool_radius * 0.4,
      "pool reaches {deepest_reach} m"
    );
    assert!(
      deepest_reach < fall.pool_radius,
      "pool reaches {deepest_reach} m"
    );

    let none = condition(
      &mut cliff_valley(),
      &RiverOptions {
        waterfalls: false,
        ..options
      },
    )
    .0;
    assert!(none.falls.is_empty());
  }

  #[test]
  fn pools_scale_with_discharge() {
    let plan_three = |height: f32, width: f32| 0.3 * height + width;
    let (small, small_depth) = pool_size(20.0, 0.6, 0.03);
    let (large, large_depth) = pool_size(20.0, 5.4, 4.0);

    assert!(small <= 0.2 * plan_three(20.0, 0.6), "{small}");
    assert!((large - plan_three(20.0, 5.4)).abs() < 1e-4);
    assert!((large_depth - 3.0).abs() < 1e-4);
    assert!(small_depth >= 0.3);
  }

  #[test]
  fn a_trickle_over_a_cliff_is_listed_but_has_no_pool() {
    let mut map = cliff_valley();
    let before = map.heights.clone();
    let (channels, _) = condition(&mut map, &cliff_options(0.01));

    assert_eq!(channels.falls.len(), 1);
    let fall = &channels.falls[0];
    assert!(fall.trickle && fall.pool_radius == 0.0);
    // Its step stays on the ribbon, as whitewater, not a fall.
    let reach = &channels.reaches[0];
    assert!(reach.points.iter().all(|point| !point.falling));
    assert!(reach.points.iter().any(|point| point.rapids > 0.0));

    // Nothing is dug below the foot: only the channel is cut there.
    let foot = (fall.foot[1].round() as u32 * 128 + fall.foot[0].round() as u32) as usize;
    assert!(before[foot] - map.heights[foot] <= 1.0);
  }

  #[test]
  fn falls_close_together_form_one_cascade() {
    // Three 8 m steps 6 samples apart.
    let mut map = map_from(128, 2.0, |x, y| {
      let steps = [40.0, 46.0, 52.0].iter().filter(|at| y >= **at).count() as f32;
      y * 0.1 - 0.5 + (x - 64.0).abs() * 0.5 + steps * 8.0
    });
    let (channels, _) = condition(&mut map, &cliff_options(3.0));
    let cascades: Vec<&Fall> = channels.falls.iter().filter(|f| !f.trickle).collect();

    assert_eq!(cascades.len(), 1, "{:?}", channels.falls);
    let cascade = cascades[0];
    assert_eq!(cascade.steps.len(), 3);
    let total: f32 = cascade
      .steps
      .iter()
      .map(|step| step.lip_level - step.foot_level)
      .sum();
    assert!(
      (cascade.height() - total).abs() < 2.0,
      "{} {}",
      cascade.height(),
      total
    );
    assert!(cascade.height() > 20.0);
  }

  #[test]
  fn plunge_pools_on_a_steep_slope_never_raise_the_ground() {
    // A 20 m cliff above a hillside falling at about 30 degrees,
    // in a valley that gathers the water.
    let shape = |x: f32, y: f32| {
      let step = if y >= 64.0 { 20.0 } else { 0.0 };
      y * 1.2 + (x - 64.0).abs() * 0.5 + step
    };
    let original = map_from(128, 2.0, shape);
    let mut map = map_from(128, 2.0, shape);
    let (channels, _) = condition(&mut map, &cliff_options(4.0));

    assert!(!channels.falls.is_empty());

    for (index, (after, before)) in map.heights.iter().zip(&original.heights).enumerate() {
      assert!(
        after <= before,
        "sample {index} rose from {before} to {after}"
      );
    }
  }

  /// A river of `discharge` m³/s running north down a straight line to a
  /// sea mouth at y = 181, its levels `level(i)` for point `i` of 300.
  fn sea_river(discharge: f32, level: impl Fn(usize) -> f32) -> RawStream {
    let n = 300;
    RawStream {
      points: (0..n).map(|i| [256.0, 480.0 - i as f32]).collect(),
      levels: (0..n)
        .map(|i| if i == n - 1 { 0.0 } else { level(i) })
        .collect(),
      discharge: vec![discharge; n],
      orders: vec![1; n],
      min_width: 0.0,
      mouth: Mouth::Sea,
      painted: false,
    }
  }

  fn shaped(raw: &RawStream, map: &HeightMap, width_scale: f32) -> Shaped {
    let options = RiverOptions {
      meanders: 0.0,
      width_scale,
      ..RiverOptions::default()
    };
    let context = ChannelContext {
      surface: &[],
      options: &options,
      seed: 5,
    };
    shape_stream(raw, None, map, 12.0, &context, 5)
  }

  #[test]
  fn a_wide_estuary_meets_the_sea_between_mouth_bars() {
    // 22 m wide upstream and up to 65 m at the mouth.
    let map = map_from(512, 12.0, |_, y| if y > 181.0 { 1.0 } else { -2.0 });
    // Too steep near the sea for a delta (0.6 %), flat enough to flare.
    let estuary = shaped(&sea_river(4.0, |i| (299 - i) as f32 * 0.072), &map, 4.0);
    assert_eq!(estuary.reaches.len(), 1, "no delta");
    let mouth = *estuary.reaches[0].points.last().unwrap();
    assert!(!estuary.fan.is_empty());
    let (mut left, mut right) = (0, 0);

    for (sample, height) in &estuary.fan {
      let (x, y) = ((sample % 512) as f32, (sample / 512) as f32);
      // Just above the sea, beside the mouth and seawards of it, never in
      // the river's own water.
      assert!(*height <= 0.3 + 1e-4 && *height > map.heights[*sample]);
      assert!(y <= mouth.y + 1.0 && (mouth.y - y) * 12.0 <= 1.2 * mouth.width);
      let across = (x - mouth.x) * 12.0;
      assert!(
        across.abs() >= 0.5 * mouth.width,
        "a bar {across} m from the mouth's middle"
      );
      left += usize::from(across < 0.0);
      right += usize::from(across > 0.0);
    }

    assert!(left > 0 && right > 0);
  }

  #[test]
  fn a_wide_river_meeting_the_sea_on_flat_ground_splits_into_a_delta() {
    // Flat land down to a coast at y = 200, then a sea deepening gently
    // offshore.
    let map = map_from(512, 12.0, |_, y| {
      if y > 200.0 {
        1.0
      } else {
        -0.2 - (200.0 - y) * 0.03
      }
    });
    let raw = sea_river(40.0, |i| 1.5 - i as f32 * 0.004);
    let shaped = shaped(&raw, &map, 4.0);
    assert!(!shaped.fan.is_empty());
    let arms = &shaped.reaches[1..];
    let starts_at = |p: &ChannelPoint| {
      arms
        .iter()
        .any(|a| a.points[0].x == p.x && a.points[0].y == p.y)
    };
    let leaves: Vec<&Reach> = arms
      .iter()
      .filter(|a| !starts_at(a.points.last().unwrap()))
      .collect();
    assert!((2..=8).contains(&leaves.len()), "{} arms", leaves.len());
    assert!(arms.len() > leaves.len(), "no arm split again");

    for leaf in &leaves {
      let end = leaf.points.last().unwrap();
      let edge = end.x < 1.5 || end.y < 1.5 || end.x > 509.5 || end.y > 509.5;
      assert!(
        end.level == 0.0 || edge,
        "an arm ends at {}, {}",
        end.x,
        end.y
      );
    }

    // Each split turns both arms 15 to 35 degrees off their parent.
    let heading = |a: &ChannelPoint, b: &ChannelPoint| (b.y - a.y).atan2(b.x - a.x).to_degrees();

    for arm in arms {
      let first = &arm.points[0];
      let parent = shaped
        .reaches
        .iter()
        .find(|r| {
          r.points
            .last()
            .is_some_and(|p| p.x == first.x && p.y == first.y)
        })
        .expect("a parent");
      let n = parent.points.len();
      let turn = (heading(&arm.points[0], &arm.points[1])
        - heading(&parent.points[n - 2], &parent.points[n - 1])
        + 540.0)
        % 360.0
        - 180.0;
      assert!(
        (14.0..=36.0).contains(&turn.abs()),
        "a split at {turn} degrees"
      );
    }

    // No two arms come within a width of each other, away from the bars
    // they leave.
    let away = |p: &ChannelPoint, arm: &Reach| {
      let s = &arm.points[0];
      length2(p.x - s.x, p.y - s.y) * 12.0 > 3.0 * s.width
    };

    for (i, a) in arms.iter().enumerate() {
      for b in &arms[i + 1..] {
        for p in a.points.iter().filter(|p| away(p, a) && away(p, b)) {
          for q in b.points.iter().filter(|q| away(q, a) && away(q, b)) {
            let apart = length2(p.x - q.x, p.y - q.y) * 12.0;
            assert!(apart >= p.width.min(q.width) * 0.99, "arms {apart} m apart");
          }
        }
      }
    }
  }

  #[test]
  fn estuaries_flare_seawards_on_flat_coasts_and_barely_on_steep_ones() {
    // 4 m³/s (5.4 m wide), too narrow for a delta.
    let map = map_from(512, 12.0, |_, y| if y > 181.0 { 1.0 } else { -2.0 });
    let flat = shaped(&sea_river(4.0, |i| 2.0 - i as f32 * 0.0067), &map, 1.0);
    let points = &flat.reaches[0].points;
    let n = points.len();
    let w0 = channel_width(4.0, 1.0);
    let mouth = points[n - 1].width;
    assert!(
      (mouth / (3.0 * w0) - 1.0).abs() <= 0.05,
      "{mouth} m at the mouth"
    );

    for pair in points.windows(2) {
      assert!(pair[1].width >= pair[0].width - 1e-4);
    }

    // The banks curve out smoothly: each turns by at most 15 degrees from
    // one row to the next.
    let bank = |i: usize| {
      let run = length2(points[i + 1].x - points[i].x, points[i + 1].y - points[i].y) * 12.0;
      (0.5 * (points[i + 1].width - points[i].width)).atan2(run)
    };

    for i in 1..n - 1 {
      let turn = (bank(i) - bank(i - 1)).abs().to_degrees();
      assert!(turn <= 15.0, "a bank turns {turn} degrees at row {i}");
    }

    let steep = shaped(&sea_river(4.0, |i| (299 - i) as f32 * 8.0), &map, 1.0);
    let end = steep.reaches[0].points.last().unwrap();
    assert!(
      end.width <= 1.5 * w0 + 1e-3,
      "{} m at a steep mouth",
      end.width
    );
  }

  #[test]
  fn steep_bedrock_reaches_are_narrower() {
    assert!((0.7..=0.8).contains(&narrowing(0.1)), "{}", narrowing(0.1));
    assert_eq!(narrowing(0.015), 1.0);
    assert_eq!(narrowing(1.0), 0.7);
    let map = map_from(512, 12.0, |_, y| y * 1.2);
    let raw = RawStream {
      mouth: Mouth::Edge,
      ..sea_river(4.0, |i| 600.0 - i as f32 * 1.2)
    };
    let points = &shaped(&raw, &map, 1.0).reaches[0].points;
    let middle = &points[points.len() / 2];
    let ratio = middle.width / channel_width(4.0, 1.0);
    assert!(
      (0.7..=0.8).contains(&ratio),
      "{ratio} of the width at {} slope",
      middle.slope
    );
  }

  #[test]
  fn a_tributary_keeps_its_own_water_to_its_join() {
    let map = branching_valleys();
    let options = RiverOptions {
      min_catchment_km2: 0.3,
      ..RiverOptions::default()
    };
    let hydrology = build_hydrology(&map, &[], &options, 9);
    let streams = raw_streams(&hydrology);
    let joins: Vec<&RawStream> = streams.iter().filter(|s| s.mouth == Mouth::Join).collect();
    assert!(!joins.is_empty());

    // Its last point lies on the main stem, which carries more, but the
    // tributary's width there is still its own.
    for stream in joins {
      let n = stream.points.len();
      assert_eq!(stream.discharge[n - 1], stream.discharge[n - 2]);
      assert_eq!(stream.orders[n - 1], stream.orders[n - 2]);
    }
  }

  #[test]
  fn tributaries_meet_their_main_stem_at_an_acute_angle_downstream() {
    let mut map = branching_valleys();
    let options = RiverOptions {
      min_catchment_km2: 0.3,
      ..RiverOptions::default()
    };
    let (channels, _) = condition(&mut map, &options);
    let joins = crate::terrain::river_metrics::junction_angles(&channels.reaches, 30.0);
    assert!(!joins.is_empty());

    for join in joins {
      assert!(
        (25.0..=90.0).contains(&join.angle),
        "a join at {} degrees",
        join.angle
      );
    }
  }

  /// A wide, gentle valley draining north to the sea, where rivers
  /// meander over a floodplain.
  fn gentle_valley() -> HeightMap {
    map_from(160, 30.0, |x, y| y * 0.12 - 1.0 + (x - 80.0).abs() * 0.4)
  }

  #[test]
  fn carving_each_sample_once_matches_the_reference_carve() {
    let fixtures = [
      (branching_valleys(), 0.3),
      (cliff_valley(), 0.005),
      (gentle_valley(), 0.2),
    ];

    for (index, (original, catchment)) in fixtures.into_iter().enumerate() {
      let options = RiverOptions {
        min_catchment_km2: catchment,
        meanders: 1.0,
        ..RiverOptions::default()
      };
      let carve = |carver: Carver| {
        let mut map = original.clone();
        let hydrology = build_hydrology(&map, &[], &options, 9);
        let streams = raw_streams(&hydrology);
        let mut record = CarveRecord::new(map.heights.len());
        let context = ChannelContext {
          surface: &[],
          options: &options,
          seed: 9,
        };
        condition_with(&mut map, &hydrology, streams, &context, &mut record, carver);
        map
      };
      let once = carve(Carver::Once(CarveScratch::new(&original)));
      let reference = carve(Carver::Reference);
      let mut changed = 0;

      for (sample, before) in original.heights.iter().enumerate() {
        let (a, b) = (once.heights[sample], reference.heights[sample]);
        // Only a delta fan or mouth bars raise ground, to half a metre
        // above the sea at most.
        let top = before.max(0.5);
        assert!(
          a <= top && b <= top,
          "fixture {index} raised sample {sample}"
        );
        assert!(
          (a - b).abs() <= 0.05,
          "fixture {index} sample {sample}: {a} against the reference {b}"
        );
        changed += usize::from(a < *before);
      }

      assert!(changed > 50, "fixture {index} carved {changed} samples");
    }
  }

  #[test]
  fn a_big_river_on_gentle_ground_flattens_its_valley_floor() {
    // A broad V valley falling 0.2 % to the sea in the north, fed from the
    // south by a river 40 m wide.
    let original = map_from(256, 30.0, |x, y| y * 0.06 - 2.0 + (x - 128.0).abs() * 0.5);
    let mut map = original.clone();
    let discharge = (40.0f32 / 2.7).powi(2);
    let options = RiverOptions {
      min_catchment_km2: 50.0,
      meanders: 0.0,
      inflow: vista_types::RiverInflows::List(vec![vista_types::RiverInflow {
        position: [0.5 * 30.0, 110.0 * 30.0],
        discharge_cubic_metres_per_second: discharge,
      }]),
      ..RiverOptions::default()
    };
    let (channels, original_heights) = condition(&mut map, &options);
    let trunk = channels
      .reaches
      .iter()
      .max_by_key(|reach| reach.points.len())
      .expect("a river");
    let mut checked = 0;

    // Each sample beside the river against its nearest point of the river:
    // within 3 w of the bank, no higher than half a metre above it.
    for point in trunk
      .points
      .iter()
      .filter(|p| p.width >= 39.0 && p.slope < 0.01)
    {
      let (x, y) = (point.x.round() as i32, point.y.round() as i32);

      for dx in -12..=12i32 {
        let index = (y * 256 + x + dx) as usize;
        let sample = [(x + dx) as f32, y as f32];
        let nearest = trunk
          .points
          .iter()
          .min_by(|a, b| {
            let da = length2(a.x - sample[0], a.y - sample[1]);
            let db = length2(b.x - sample[0], b.y - sample[1]);
            da.total_cmp(&db)
          })
          .unwrap();
        let distance = length2(nearest.x - sample[0], nearest.y - sample[1]) * 30.0;
        let bank = nearest.level + 0.25 * nearest.depth;
        let r = (0.5 * nearest.width).max(0.75 * 30.0);

        if nearest.width < 39.0 || distance <= r || distance > r + 3.0 * nearest.width {
          continue;
        }

        if original.heights[index] > 0.0 {
          assert!(
            map.heights[index] <= bank + 0.5 + 1e-3,
            "{} m above the bank at {} m",
            map.heights[index] - bank,
            distance
          );
          checked += 1;
        }
      }
    }

    assert!(checked > 100, "checked {checked} samples");

    for (index, height) in original_heights {
      map.heights[index] = height;
    }

    assert_eq!(map.heights, original.heights);
  }

  /// Condition `streams`, main stems first, into `map`, returning the
  /// channels and the heights the carve replaced.
  fn condition_streams(
    map: &mut HeightMap,
    streams: Vec<RawStream>,
    options: &RiverOptions,
  ) -> (Channels, Vec<(usize, f32)>) {
    let hydrology = build_hydrology(map, &[], options, 9);
    let mut record = CarveRecord::new(map.heights.len());
    let context = ChannelContext {
      surface: &[],
      options,
      seed: 3,
    };
    let channels = condition_channels(map, &hydrology, streams, &context, &mut record);
    (channels, record.into_original())
  }

  /// A straight valley along x at 12 m samples falling 1.5 %, with a flat
  /// floor `floor` widths wide for its 60 m³/s river (w about 21 m).
  fn braided_valley(floor: f32) -> (HeightMap, RawStream) {
    let half = floor * channel_width(60.0, 1.0) / 12.0 / 2.0;
    let fall = |x: f32| 100.0 - x * 12.0 * 0.015;
    let map = map_from(512, 12.0, |x, y| {
      fall(x) + 0.05 + ((y - 256.0).abs() - half).max(0.0) * 4.0
    });
    let n = 500;
    let raw = RawStream {
      points: (0..n).map(|i| [6.0 + i as f32, 256.0]).collect(),
      levels: (0..n).map(|i| fall(6.0 + i as f32)).collect(),
      discharge: vec![60.0; n],
      orders: vec![3; n],
      min_width: 0.0,
      mouth: Mouth::Edge,
      painted: false,
    };
    (map, raw)
  }

  #[test]
  fn steep_wide_open_valleys_braid_into_threads_between_gravel_bars() {
    let braided = |floor: f32, braiding: f32| {
      let (mut map, raw) = braided_valley(floor);
      let options = RiverOptions {
        braiding,
        ..RiverOptions::default()
      };
      let (channels, _) = condition_streams(&mut map, vec![raw], &options);
      (map, channels)
    };
    let (map, channels) = braided(8.0, 1.0);
    let of = |kind: ReachKind| {
      channels
        .reaches
        .iter()
        .filter(move |reach| reach.kind == kind)
    };
    let belt = &of(ReachKind::Belt).next().expect("no belt").points;
    let threads = of(ReachKind::Thread).count();
    assert!((2..=4).contains(&threads), "{threads} threads");

    // Every thread stays within its belt.
    for p in of(ReachKind::Thread).flat_map(|reach| &reach.points) {
      let q = belt
        .iter()
        .min_by(|a, b| length2(a.x - p.x, a.y - p.y).total_cmp(&length2(b.x - p.x, b.y - p.y)))
        .unwrap();
      assert!(
        length2(q.x - p.x, q.y - p.y) * 12.0 <= 0.5 * q.width + 0.5,
        "a thread leaves its belt at {}, {}",
        p.x,
        p.y
      );
    }

    // Gravel bars between the threads.
    let bed = crate::render::water::bed_materials(&map, &channels.reaches, &[], &channels.mask);
    let gravel = bed
      .iter()
      .filter(|(_, weights)| weights[0] > weights[1].max(weights[2]).max(weights[3]))
      .count();
    assert!(gravel > 100, "{gravel} gravel samples");

    for (floor, braiding) in [(2.0, 1.0), (8.0, 0.0)] {
      let (_, channels) = braided(floor, braiding);
      assert!(
        channels
          .reaches
          .iter()
          .all(|reach| reach.kind == ReachKind::Main),
        "braided with a floor of {floor} w at {braiding}"
      );
    }
  }

  #[test]
  fn bends_cut_their_outer_bank_and_build_a_point_bar() {
    // A V-shaped section 20 m wide: r is 10 m.
    let shape = ChannelShape::new(10.0, 20.0, 1.0, 0.0, false);
    let at = |distance: f32, bend: f32| shape.target(1000.0, 10.0, 2.0, distance, bend);
    let wall = |bend: f32| (at(30.0, bend) - at(10.0, bend)) / 20.0;
    assert!(
      wall(0.8) >= 1.5 * wall(-0.8),
      "{} against {}",
      wall(0.8),
      wall(-0.8)
    );
    let deepest = |bend: f32| {
      (0..=100)
        .map(|k| at(k as f32 * 0.1, bend))
        .fold(f32::INFINITY, f32::min)
    };
    assert!(deepest(0.8) < deepest(-0.8));
    // The inner bank is carved less than a straight reach's, never more.
    assert!((0..=400).all(|k| at(k as f32 * 0.1, -0.8) >= at(k as f32 * 0.1, 0.0) - 1e-4));
  }

  #[test]
  fn meander_belts_are_floored_up_to_their_valley_walls() {
    let ground = |x: f32, y: f32| 10.0 + 3.0 * ((x * 0.37).sin() * (y * 0.23).cos()).abs();
    let mut map = map_from(64, 12.0, ground);
    let before = map.heights.clone();
    let point = |x: f32, y: f32| ChannelPoint {
      x,
      y,
      level: 10.0,
      bed: 9.0,
      width: 24.0,
      depth: 1.0,
      discharge: 80.0,
      slope: 0.001,
      speed: 0.5,
      curvature: 0.0,
      celsius: 12.0,
      rapids: 0.0,
      falling: false,
      order: 3,
    };
    let river: Vec<ChannelPoint> = (0..40).map(|i| point(12.0 + i as f32, 32.0)).collect();
    let snapshot: Vec<ChannelPoint> = (0..40).map(|i| point(12.0 + i as f32, 28.0)).collect();
    let mut record = CarveRecord::new(map.heights.len());
    let mut near = Near {
      flags: vec![false; map.heights.len()],
      set: Vec::new(),
    };
    carve_belt(&mut map, &[river, snapshot], 12.0, &mut record, &mut near);
    assert!(near.set.is_empty() && near.flags.iter().all(|flag| !flag));
    let (floor, top) = (10.55, 11.5);

    for y in 26..=34 {
      for x in 12..=51 {
        let index = y * 64 + x;
        assert!(
          map.heights[index] <= floor + 1e-4 || before[index] > top,
          "{} at {x}, {y}",
          map.heights[index]
        );
      }
    }

    // A swale along the old channel, away from the river.
    let swales = (12..=51)
      .filter(|x| before[28 * 64 + x] <= top && map.heights[28 * 64 + x] <= floor - 0.149)
      .count();
    assert!(swales > 10, "{swales} swale samples");

    for (index, height) in record.into_original() {
      map.heights[index] = height;
    }

    assert_eq!(map.heights, before);
  }

  #[test]
  fn the_carve_record_restores_exactly() {
    let mut map = cliff_valley();
    let before = map.heights.clone();
    let options = cliff_options(4.0);
    let (_, original) = condition(&mut map, &options);
    assert!(!original.is_empty());

    for (index, height) in original {
      map.heights[index] = height;
    }

    assert_eq!(map.heights, before);

    // Migrated reaches with their belt floor, swales and oxbows, and a
    // braided belt with its threads.
    let options = RiverOptions {
      meanders: 1.0,
      meander_maturity: 1.0,
      ..RiverOptions::default()
    };
    let fixtures = [
      (floodplain_map(0.001, 40.0), floodplain_river(0.001)),
      braided_valley(8.0),
    ];

    for (mut map, raw) in fixtures {
      let before = map.heights.clone();
      let (channels, original) = condition_streams(&mut map, vec![raw], &options);
      assert!(!channels.oxbows.is_empty() || channels.reaches.len() > 1);

      for (index, height) in original {
        map.heights[index] = height;
      }

      assert_eq!(map.heights, before);
    }
  }
}
