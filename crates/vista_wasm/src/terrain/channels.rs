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

use crate::maths::{hash_u64, length2, smoothstep};
use crate::terrain::biomes::SurfaceSample;
use crate::terrain::heightmap::HeightMap;
use crate::terrain::hydrology::{Hydrology, Mouth, NO_LAKE};

/// Manning roughness of a natural channel.
const MANNING_N: f32 = 0.035;

/// Gravity, in metres per second squared.
pub const GRAVITY: f32 = 9.81;

/// Valley slope above which channels are cut as a V.
const V_SLOPE: f32 = 0.06;

/// Valley slope below which channels are flat-bottomed with a floodplain.
const FLAT_SLOPE: f32 = 0.02;

/// Meanders form below this valley slope.
const MEANDER_SLOPE: f32 = 0.015;

/// Meanders form on channels wider than this, in metres.
const MEANDER_WIDTH: f32 = 4.0;

/// Deltas form on channels wider than this, in metres.
const DELTA_WIDTH: f32 = 8.0;

/// Deltas form where the last reach is flatter than this.
const DELTA_SLOPE: f32 = 0.005;

/// A step lower than this is a rapid, not a fall.
const MIN_FALL_METRES: f32 = 3.0;

/// Channel width in metres for a discharge, before `widthScale`.
pub fn channel_width(discharge: f32, width_scale: f32) -> f32 {
  (2.7 * discharge.max(0.0).sqrt() * width_scale).clamp(0.6, 400.0)
}

/// Channel depth in metres for a discharge.
pub fn channel_depth(discharge: f32) -> f32 {
  (0.35 * discharge.max(0.0).powf(0.4)).clamp(0.6, 400.0)
}

/// Mean flow speed from Manning's equation, with the hydraulic radius
/// taken as the depth, clamped to 0.2 to 6 m/s.
pub fn manning_speed(depth: f32, slope: f32) -> f32 {
  (depth.powf(2.0 / 3.0) * slope.max(1.0e-5).sqrt() / MANNING_N).clamp(0.2, 6.0)
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
}

/// A channel centreline from upstream to downstream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reach {
  /// Points from upstream to downstream.
  pub points: Vec<ChannelPoint>,
}

/// A waterfall where a channel drops over a step.
#[derive(Clone, Copy, Debug, PartialEq)]
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
}

impl Fall {
  /// Height of the drop, in metres.
  pub fn height(&self) -> f32 {
    self.lip_level - self.foot_level
  }
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
  /// Channel width at least this, in metres (painted rivers).
  pub min_width: f32,
  /// Where it ends.
  pub mouth: Mouth,
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

      // A sea or lake cell carries everything that reaches it; the
      // stream's own discharge is the last land cell's.
      if matches!(stream.mouth, Mouth::Sea | Mouth::Lake(_)) && n >= 2 {
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
        min_width: 0.0,
        mouth: stream.mouth,
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
  let mut channels = Channels {
    mask: vec![false; map.heights.len()],
    ..Channels::default()
  };
  let metres = map.metadata.metres_per_sample.max(0.001);
  // Water level of every point already conditioned, so tributaries meet
  // the stream they join at its level.
  let mut joined = vec![f32::NAN; map.heights.len()];
  let map_width = map.metadata.width;
  let slot = |x: f32, y: f32| (y.round() as u32 * map_width + x.round() as u32) as usize;

  for (index, mut raw) in streams.into_iter().enumerate() {
    if raw.points.len() < 2 {
      continue;
    }

    if raw.mouth == Mouth::Join {
      let last = raw.points[raw.points.len() - 1];

      let level = joined[slot(last[0], last[1])];

      if !level.is_nan() {
        let n = raw.levels.len();
        raw.levels[n - 1] = raw.levels[n - 1].min(level);
      }
    }

    let seed = hash_u64(context.seed ^ (index as u64).wrapping_mul(0x2545_f491_4f6c_dd1d));
    let Shaped {
      mut reaches,
      falls,
      oxbows,
      fan,
    } = shape_stream(&raw, map, metres, context, seed);

    for (sample, target) in fan {
      if target > map.heights[sample] {
        record.set(map, sample, target);
      }
    }

    for reach in &reaches {
      for point in &reach.points {
        let at = slot(point.x, point.y);

        if joined[at].is_nan() {
          joined[at] = point.level;
        }
      }
    }

    // Pools first, so the channel below cuts its outlet through the lip.
    for fall in &falls {
      carve_pool(map, fall, metres, record, &mut channels.mask);
    }

    for reach in &reaches {
      carve_reach(
        map,
        hydrology,
        context.surface,
        reach,
        metres,
        record,
        &mut channels.mask,
      );
    }

    for oxbow in &oxbows {
      carve_oxbow(map, oxbow, metres, record);
    }

    channels.reaches.append(&mut reaches);
    channels.falls.extend(falls);
    channels.oxbows.extend(oxbows);
  }

  channels
}

/// Arc length along a polyline in metres, at each point.
fn arc_lengths(points: &[[f32; 2]], metres: f32) -> Vec<f32> {
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
fn kinoshita_table() -> [f32; 64] {
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

fn kinoshita(table: &[f32; 64], phase: f32) -> f32 {
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
  /// Delta fan samples and the heights they are raised to.
  fan: Vec<(usize, f32)>,
}

/// Condition one stream.
fn shape_stream(
  raw: &RawStream,
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

  let in_step = |i: usize| steps.iter().any(|(a, b)| i >= *a && i < *b);
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
        width: widths[i],
        depth,
        discharge: raw.discharge[i],
        slope,
        speed: manning_speed(depth, slope),
        curvature: 0.0,
        celsius: celsius(raw.points[i][0], raw.points[i][1]),
        rapids: rapids[i],
        falling: steps.iter().any(|(a, b)| i >= *a && i <= *b),
      }
    })
    .collect();

  let falls: Vec<Fall> = steps
    .iter()
    .map(|(a, b)| {
      let lip = points[*a];
      let foot = points[*b];
      let dx = foot.x - lip.x;
      let dy = foot.y - lip.y;
      let length = length2(dx, dy).max(1e-4);
      let height = lip.level - foot.level;
      // The water arrives at the lip at the speed of the reach above.
      let approach = points[a.saturating_sub(1)].speed;

      Fall {
        lip: [lip.x, lip.y],
        lip_level: lip.level,
        foot: [foot.x, foot.y],
        foot_level: foot.level,
        direction: [dx / length, dy / length],
        width: lip.width,
        discharge: lip.discharge,
        speed: approach,
        pool_radius: 0.3 * height + foot.width,
        pool_depth: 0.15 * height,
        celsius: lip.celsius,
      }
    })
    .collect();

  let mut oxbows = Vec::new();
  // Distance along the stream to the nearest step, in metres.
  let far_from_steps: Vec<f32> = (0..n)
    .map(|i| {
      steps
        .iter()
        .map(|(a, b)| (s[*a] - s[i]).max(s[i] - s[*b]).max(0.0))
        .fold(f32::INFINITY, f32::min)
    })
    .collect();

  if options.meanders > 0.0 {
    points = meander(
      &points,
      &s,
      &far_from_steps,
      options.meanders.clamp(0.0, 1.0),
      raw.mouth,
      map,
      metres,
      seed,
      &mut oxbows,
    );
  }

  set_curvature(&mut points, metres);

  if raw.mouth == Mouth::Sea {
    if let Some(delta) = delta(&points, map, metres, seed) {
      let mut reaches = vec![Reach {
        points: delta.trunk,
      }];
      reaches.extend(delta.arms.into_iter().map(|points| Reach { points }));
      return Shaped {
        reaches,
        falls,
        oxbows,
        fan: delta.fan,
      };
    }
  }

  Shaped {
    reaches: vec![Reach { points }],
    falls,
    oxbows,
    fan: Vec::new(),
  }
}

/// Displace lowland reaches into meanders. Amplitude grows to 2.5 w on
/// flat ground (times `strength`), the wavelength is 11 w, and the ends
/// of the stream, its steps, and its last 8 w before the sea stay pinned.
/// The sharpest tenth of the loops leave oxbow lakes beside them.
#[allow(clippy::too_many_arguments)]
fn meander(
  points: &[ChannelPoint],
  s: &[f32],
  far_from_steps: &[f32],
  strength: f32,
  mouth: Mouth,
  map: &HeightMap,
  metres: f32,
  seed: u64,
  oxbows: &mut Vec<Oxbow>,
) -> Vec<ChannelPoint> {
  let n = points.len();
  let total = s[n - 1];
  let end_pin = |p: &ChannelPoint| {
    let wavelength = 11.0 * p.width;
    wavelength * 0.5
      + if mouth == Mouth::Sea {
        8.0 * p.width
      } else {
        0.0
      }
  };
  let amplitude: Vec<f32> = (0..n)
    .map(|i| {
      let p = &points[i];
      let wavelength = 11.0 * p.width;
      let flat = 1.0 - smoothstep((p.slope - 0.004) / (MEANDER_SLOPE - 0.004));
      let wide = smoothstep((p.width - MEANDER_WIDTH) / 1.0);
      let pin = smoothstep(s[i] / (wavelength * 0.5))
        * smoothstep((total - s[i]) / end_pin(p))
        * smoothstep(far_from_steps[i] / (wavelength * 0.5));
      2.5 * p.width * strength * flat * wide * pin
    })
    .collect();

  if amplitude.iter().all(|a| *a < metres * 0.25) {
    return points.to_vec();
  }

  let table = kinoshita_table();
  let phase0 = (seed % 1000) as f32 / 1000.0;
  let side = if (seed >> 12) & 1 == 0 { 1.0 } else { -1.0 };
  let mut dense = Vec::new();
  let mut offsets = Vec::new();
  let mut phases = Vec::new();
  let mut phase = phase0;
  let mut along = 0.0f32;
  let mut i = 0;

  while along <= total {
    while i + 1 < n - 1 && s[i + 1] < along {
      i += 1;
    }

    let j = (i + 1).min(n - 1);
    let span = (s[j] - s[i]).max(1e-4);
    let t = ((along - s[i]) / span).clamp(0.0, 1.0);
    let (a, b) = (&points[i], &points[j]);
    let lerp = |u: f32, v: f32| u + (v - u) * t;
    let tangent = [b.x - a.x, b.y - a.y];
    let length = length2(tangent[0], tangent[1]).max(1e-6);
    let normal = [-tangent[1] / length, tangent[0] / length];
    let width = lerp(a.width, b.width);
    let offset = lerp(amplitude[i], amplitude[j]) * kinoshita(&table, phase) * side;
    let mut point = *a;
    point.x =
      (lerp(a.x, b.x) + normal[0] * offset / metres).clamp(0.0, (map.metadata.width - 1) as f32);
    point.y =
      (lerp(a.y, b.y) + normal[1] * offset / metres).clamp(0.0, (map.metadata.height - 1) as f32);
    point.level = lerp(a.level, b.level);
    point.width = width;
    point.depth = lerp(a.depth, b.depth);
    point.bed = point.level - point.depth;
    point.discharge = lerp(a.discharge, b.discharge);
    point.slope = lerp(a.slope, b.slope);
    point.speed = lerp(a.speed, b.speed);
    point.rapids = lerp(a.rapids, b.rapids);
    point.falling = a.falling && (t < 1e-6 || b.falling);
    dense.push(point);
    offsets.push((offset, normal));
    phases.push(phase);

    if along >= total {
      break;
    }

    let step = (metres * 0.5).min(11.0 * width / 24.0).max(0.05);
    let next = (along + step).min(total);
    phase += (next - along) / (11.0 * width);
    along = next;
  }

  // The level never rises, whatever the loops do.
  for k in 1..dense.len() {
    dense[k].level = dense[k].level.min(dense[k - 1].level);
    dense[k].bed = dense[k].bed.min(dense[k - 1].bed);
  }

  // Loop apexes: where the offset stops growing and turns back.
  let mut loops = Vec::new();

  for k in 1..dense.len().saturating_sub(1) {
    let (o, _) = offsets[k];
    let (before, _) = offsets[k - 1];
    let (after, _) = offsets[k + 1];

    if o.abs() > before.abs() && o.abs() >= after.abs() && o.abs() > dense[k].width {
      let wavelength = 11.0 * dense[k].width;
      loops.push((o.abs() / (wavelength * wavelength), k));
    }
  }

  // The sharpest tenth.
  let keep = (loops.len() as f32 * 0.1).round() as usize;

  for _ in 0..keep {
    let Some((slot, _)) = loops
      .iter()
      .enumerate()
      .max_by(|a, b| a.1 .0.total_cmp(&b.1 .0))
    else {
      break;
    };
    let (_, k) = loops.swap_remove(slot);
    let apex = dense[k];
    let (offset, _) = offsets[k];
    let outward = offset.signum();
    let shift = (2.0 * apex.width + 0.3 * offset.abs()) / metres * outward;
    let points: Vec<[f32; 2]> = (0..dense.len())
      .filter(|m| (phases[*m] - phases[k]).abs() < 0.12)
      .map(|m| {
        let (_, nm) = offsets[m];
        [dense[m].x + nm[0] * shift, dense[m].y + nm[1] * shift]
      })
      .collect();

    if points.len() >= 3 {
      oxbows.push(Oxbow {
        points,
        width: apex.width,
        surface: apex.level,
        celsius: apex.celsius,
      });
    }
  }

  dense
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

/// Split a large river meeting the sea on flat ground into two or three
/// distributaries fanning out at ±25 degrees over its last 8 w, and
/// deposit a low fan between them.
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
  let mouth = points[n - 1];
  let slope = (head.level - mouth.level) / (total - s[split]).max(1.0);

  if slope >= DELTA_SLOPE {
    return None;
  }

  let dx = mouth.x - head.x;
  let dy = mouth.y - head.y;
  let length = length2(dx, dy).max(1e-4);
  let direction = [dx / length, dy / length];
  let count = 2 + (seed >> 20) as usize % 2;
  let angles: &[f32] = if count == 2 {
    &[-25.0, 25.0]
  } else {
    &[-25.0, 0.0, 25.0]
  };
  let sea = map.metadata.sea_level_metres;

  // The fan: a low sector of silt between the arms, dipping under the sea
  // at its rim.
  let radius = reach * 1.2 / metres;
  let (w, h) = (map.metadata.width as i32, map.metadata.height as i32);
  let r = radius.ceil() as i32;
  let mut raised = Vec::new();

  for y in (head.y as i32 - r).max(0)..=(head.y as i32 + r).min(h - 1) {
    for x in (head.x as i32 - r).max(0)..=(head.x as i32 + r).min(w - 1) {
      let ox = x as f32 - head.x;
      let oy = y as f32 - head.y;
      let distance = length2(ox, oy);

      if distance > radius || distance < 1e-3 {
        continue;
      }

      let cos = (ox * direction[0] + oy * direction[1]) / distance;

      if cos < 30f32.to_radians().cos() {
        continue;
      }

      let index = (y * w + x) as usize;
      let target = sea + 0.5 - (distance / radius - 0.85).max(0.0) * 6.0;

      if !map.no_data[index] && map.heights[index] < target {
        raised.push((index, target));
      }
    }
  }

  let trunk = points[..=split].to_vec();
  let q = head.discharge / count as f32;
  let width = channel_width(q, head.width / channel_width(head.discharge, 1.0));
  let depth = channel_depth(q);
  let arm_length = reach * 1.15 / metres;
  let arms = angles
    .iter()
    .map(|angle| {
      let (sin, cos) = angle.to_radians().sin_cos();
      let d = [
        direction[0] * cos - direction[1] * sin,
        direction[0] * sin + direction[1] * cos,
      ];
      let steps = ((arm_length / 0.5).ceil() as usize).max(4);

      (0..=steps)
        .map(|k| {
          let t = k as f32 / steps as f32;
          // A gentle bend away from the trunk's line.
          let bend = (t * std::f32::consts::PI).sin() * 0.08 * arm_length * angle.signum();
          let x = head.x + d[0] * arm_length * t - d[1] * bend;
          let y = head.y + d[1] * arm_length * t + d[0] * bend;
          let level = head.level + (sea - head.level) * t;
          ChannelPoint {
            x: x.clamp(0.0, (w - 1) as f32),
            y: y.clamp(0.0, (h - 1) as f32),
            level,
            bed: level - depth,
            width,
            depth,
            discharge: q,
            slope: head.slope,
            speed: manning_speed(depth, head.slope),
            curvature: 0.0,
            celsius: head.celsius,
            rapids: 0.0,
            falling: false,
          }
        })
        .collect()
    })
    .collect();

  Some(Delta {
    trunk,
    arms,
    fan: raised,
  })
}

/// Cut one reach into the map.
fn carve_reach(
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
    let steep = smoothstep((a.slope.max(b.slope) - FLAT_SLOPE) / (V_SLOPE - FLAT_SLOPE));
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
        let distance = length2(px - segment[0] * t, py - segment[1] * t) * metres;

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
        let v_shape = if distance <= r {
          bed + depth * distance / r
        } else {
          level + (distance - r)
        };
        let bank = level + 0.25 * depth;
        let trapezoid = if distance <= 0.7 * r {
          bed
        } else if distance <= r {
          bed + (bank - bed) * (distance - 0.7 * r) / (0.3 * r)
        } else {
          bank + (ground - bank) * smoothstep((distance - r) / (4.0 * w))
        };
        let target = trapezoid + (v_shape - trapezoid) * steep;
        record.lower(map, index, target);

        if distance <= (0.65 * w).max(0.5 * metres) {
          mask[index] = true;
        }
      }
    }
  }
}

/// Shape a plunge pool: a bowl of the fall's pool radius and depth, with
/// its rim at the water level all round.
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

      record.set(
        map,
        index,
        fall.foot_level - fall.pool_depth * (1.0 - d * d),
      );
      mask[index] = true;
    }
  }
}

/// Carve an oxbow: a flat hollow 0.6 d below its surface.
fn carve_oxbow(map: &mut HeightMap, oxbow: &Oxbow, metres: f32, record: &mut CarveRecord) {
  let width = map.metadata.width as i32;
  let height = map.metadata.height as i32;
  let depth = 0.6 * channel_depth((oxbow.width / 2.7).powi(2));
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

  fn plain_stream(meanders: f32) -> Vec<ChannelPoint> {
    let mut map = map_from(64, 12.0, |_, _| 5.0);
    let _ = &mut map;
    let n = 800;
    let raw = RawStream {
      points: (0..n).map(|i| [2.0 + i as f32 * 0.075, 32.0]).collect(),
      levels: (0..n).map(|i| 5.0 - i as f32 * 0.0005).collect(),
      discharge: vec![12.0; n],
      min_width: 0.0,
      mouth: Mouth::Edge,
    };
    let options = RiverOptions {
      meanders,
      ..RiverOptions::default()
    };
    let context = ChannelContext {
      surface: &[],
      options: &options,
      seed: 3,
    };
    let big = map_from(1024, 12.0, |_, _| 5.0);
    let mut shaped = shape_stream(&raw, &big, 12.0, &context, 3);
    shaped.reaches.remove(0).points
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
  fn flat_plains_meander_and_straight_options_do_not() {
    let meandering = plain_stream(1.0);
    let straight = plain_stream(0.0);

    assert!(
      sinuosity(&meandering) >= 1.3,
      "sinuosity {}",
      sinuosity(&meandering)
    );
    assert!(
      sinuosity(&straight) <= 1.05,
      "sinuosity {}",
      sinuosity(&straight)
    );
    // The ends stay pinned so joins stay connected.
    assert!((meandering[0].y - 32.0).abs() < 0.01);
    assert!((meandering[meandering.len() - 1].y - 32.0).abs() < 0.01);
  }

  /// A valley draining north with a 20 m cliff across it at y = 64.
  fn cliff_valley() -> HeightMap {
    map_from(128, 2.0, |x, y| {
      let step = if y >= 64.0 { 20.0 } else { 0.0 };
      y * 0.1 - 0.5 + (x - 64.0).abs() * 0.5 + step
    })
  }

  #[test]
  fn a_cliff_step_makes_one_waterfall_with_a_plunge_pool() {
    let mut map = cliff_valley();
    let options = RiverOptions {
      min_catchment_km2: 0.005,
      ..RiverOptions::default()
    };
    let (channels, _) = condition(&mut map, &options);

    assert_eq!(channels.falls.len(), 1, "falls {:?}", channels.falls);
    let fall = channels.falls[0];
    assert!(
      (fall.height() - 20.0).abs() <= 2.0,
      "height {}",
      fall.height()
    );
    assert!((fall.pool_radius - (0.3 * fall.height() + fall.width)).abs() < 1e-4);

    // The pool is cut to about its radius around the foot, and no further.
    let metres = map.metadata.metres_per_sample;
    let mut deepest_reach = 0.0f32;

    for y in 0..128u32 {
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
  fn a_wide_river_meeting_the_sea_on_flat_ground_splits_into_a_delta() {
    // Flat land down to a coast at y = 200, then shallow sea.
    let map = map_from(512, 12.0, |_, y| if y > 200.0 { 1.0 } else { -2.0 });
    let n = 300;
    let raw = RawStream {
      points: (0..n).map(|i| [256.0, 480.0 - i as f32]).collect(),
      levels: (0..n)
        .map(|i| {
          if i == n - 1 {
            0.0
          } else {
            1.5 - i as f32 * 0.004
          }
        })
        .collect(),
      discharge: vec![40.0; n],
      min_width: 0.0,
      mouth: Mouth::Sea,
    };
    let options = RiverOptions {
      meanders: 0.0,
      ..RiverOptions::default()
    };
    let context = ChannelContext {
      surface: &[],
      options: &options,
      seed: 5,
    };
    let shaped = shape_stream(&raw, &map, 12.0, &context, 5);

    assert!(shaped.reaches.len() == 3 || shaped.reaches.len() == 4);
    assert!(!shaped.fan.is_empty());
    let trunk_end = *shaped.reaches[0].points.last().unwrap();

    for arm in &shaped.reaches[1..] {
      let end = arm.points.last().unwrap();
      let angle = (end.x - trunk_end.x)
        .atan2(-(end.y - trunk_end.y))
        .to_degrees();
      assert!(angle.abs() <= 26.0, "arm at {angle} degrees");
      assert_eq!(end.level, 0.0);
    }
  }

  #[test]
  fn the_carve_record_restores_exactly() {
    let mut map = cliff_valley();
    let before = map.heights.clone();
    let options = RiverOptions {
      min_catchment_km2: 0.005,
      ..RiverOptions::default()
    };
    let (_, original) = condition(&mut map, &options);
    assert!(!original.is_empty());

    for (index, height) in original {
      map.heights[index] = height;
    }

    assert_eq!(map.heights, before);
  }
}
