//! Continuous river centrelines.
//!
//! The hydrology routes water from cell to cell, so a stream arrives here
//! as a chain of sample positions, each segment along one of the eight
//! grid directions. This stage turns that chain into the curve the river
//! would follow: each point slides across the flow onto the lowest ground
//! nearby (the thalweg), Taubin smoothing (Taubin, 1995) folds staircases
//! onto the valley line without shrinking its bends, and a centripetal
//! Catmull-Rom spline (Yuksel, Schaefer and Keyser, 2011) is resampled at
//! an even spacing. Every move is checked against the ground, so the river
//! never climbs out of its valley or across a spur. Pinned points (heads,
//! mouths, falls, the map edge, joins) never move.

use crate::maths::length2;
use crate::terrain::channels::{height_at, ChannelPoint};
use crate::terrain::heightmap::HeightMap;

/// Taubin smoothing's shrinking and inflating factors.
const TAUBIN_LAMBDA: f32 = 0.5;
const TAUBIN_MU: f32 = -0.53;

/// Taubin iterations (each a shrinking and an inflating pass).
const TAUBIN_ITERATIONS: usize = 10;

/// Most passes easing bends tighter than 1.5 max(w, g).
const BEND_PASSES: usize = 24;

/// Radius, in samples, of the circle through three points; infinite on a
/// straight line.
pub(crate) fn radius(a: [f32; 2], p: [f32; 2], b: [f32; 2]) -> f32 {
  let cross = ((p[0] - a[0]) * (b[1] - a[1]) - (p[1] - a[1]) * (b[0] - a[0])).abs();

  if cross < 1e-9 {
    return f32::INFINITY;
  }

  length2(p[0] - a[0], p[1] - a[1])
    * length2(b[0] - p[0], b[1] - p[1])
    * length2(b[0] - a[0], b[1] - a[1])
    / (2.0 * cross)
}

/// How far the thalweg search looks either side of a point, in samples,
/// and its step.
const THALWEG_REACH: f32 = 0.7;
const THALWEG_STEP: f32 = 0.175;

/// A centreline and which of its points are pinned.
#[derive(Clone, Debug, Default)]
pub struct Smoothed {
  /// Points from upstream to downstream.
  pub points: Vec<ChannelPoint>,
  /// Whether each point is pinned.
  pub pins: Vec<bool>,
}

/// The spacing, in metres, a centreline is resampled at for a channel `w`
/// metres wide on a grid `g` metres apart. Where meanders can show
/// (11 w at least 3 g) it is half the width, within a quarter and half a
/// sample, so a meander wavelength has at least ten points and the
/// sharpest bend the migration allows turns by under 30 degrees a point.
/// Narrower streams are resampled every half sample: their bends are at
/// least a sample in radius, so each point turns by under 30 degrees too,
/// and finer points would only cost vertices.
pub fn spacing(width: f32, metres: f32) -> f32 {
  if 11.0 * width < 3.0 * metres {
    0.5 * metres
  } else {
    (0.5 * width).clamp(0.25 * metres, 0.5 * metres)
  }
}

/// Whether the ground under `(x, y)` is no higher than `point`'s channel
/// may climb there: `level + max(d, 1 m)`, plus `0.75 S g` on a reach of
/// slope `S` (at most 1), since the level itself changes by that much
/// within the 0.75 samples a point may move along a steep reach. On
/// gentle rivers the allowance is the depth; in mountain cascades a few
/// metres, still far short of a spur.
pub fn ground_allows(map: &HeightMap, x: f32, y: f32, point: &ChannelPoint, metres: f32) -> bool {
  height_at(map, x, y) <= ceiling(point, metres)
}

/// The highest ground `point`'s channel may climb (see [`ground_allows`]).
fn ceiling(point: &ChannelPoint, metres: f32) -> f32 {
  point.level + point.depth.max(1.0) + 0.75 * point.slope.min(1.0) * metres
}

/// The valley floor width in metres at each point: the distance across the
/// centreline between the points where the ground first rises above
/// `level + max(1.5 d, 1 m)`, scanned out to 12 w each side in half-sample
/// steps. Measured every fourth point and interpolated between.
pub fn floor_widths(points: &[ChannelPoint], map: &HeightMap, metres: f32) -> Vec<f32> {
  let n = points.len();

  if n == 0 {
    return Vec::new();
  }

  let measure = |i: usize| {
    let p = &points[i];
    let (a, b) = (&points[i.saturating_sub(1)], &points[(i + 1).min(n - 1)]);
    let (tx, ty) = (b.x - a.x, b.y - a.y);
    let length = length2(tx, ty);

    if length < 1e-6 {
      return 0.0;
    }

    let normal = [-ty / length, tx / length];
    let top = p.level + (1.5 * p.depth).max(1.0);
    let steps = ((12.0 * p.width / metres) / 0.5).ceil().max(1.0) as i32;
    let mut width = 0.0;

    for side in [-1.0f32, 1.0] {
      let mut reached = steps as f32 * 0.5;

      for k in 1..=steps {
        let offset = k as f32 * 0.5 * side;
        let (x, y) = (p.x + normal[0] * offset, p.y + normal[1] * offset);

        if height_at(map, x, y) > top {
          reached = (k as f32 - 0.5) * 0.5;
          break;
        }
      }

      width += reached * metres;
    }

    width
  };
  let mut measured: Vec<(usize, f32)> = (0..n).step_by(4).map(|i| (i, measure(i))).collect();

  if measured.last().is_some_and(|(i, _)| *i != n - 1) {
    measured.push((n - 1, measure(n - 1)));
  }

  let mut widths = vec![0.0; n];

  for pair in measured.windows(2) {
    let ((a, wa), (b, wb)) = (pair[0], pair[1]);

    for (i, slot) in widths.iter_mut().enumerate().take(b + 1).skip(a) {
      let t = (i - a) as f32 / (b - a).max(1) as f32;
      *slot = wa + (wb - wa) * t;
    }
  }

  if measured.len() == 1 {
    widths[0] = measured[0].1;
  }

  widths
}

/// One point of a centripetal Catmull-Rom segment from `p1` to `p2`.
pub fn catmull_rom(p: [[f32; 2]; 4], t: f32) -> [f32; 2] {
  // Knot spacing is the square root of the chord length (alpha = 0.5).
  let knot = |a: [f32; 2], b: [f32; 2]| length2(b[0] - a[0], b[1] - a[1]).sqrt().max(1e-4);
  let t0 = 0.0;
  let t1 = t0 + knot(p[0], p[1]);
  let t2 = t1 + knot(p[1], p[2]);
  let t3 = t2 + knot(p[2], p[3]);
  let u = t1 + (t2 - t1) * t;
  let lerp = |a: [f32; 2], b: [f32; 2], ta: f32, tb: f32| {
    let w = (u - ta) / (tb - ta);
    [a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w]
  };
  let a1 = lerp(p[0], p[1], t0, t1);
  let a2 = lerp(p[1], p[2], t1, t2);
  let a3 = lerp(p[2], p[3], t2, t3);
  let b1 = lerp(a1, a2, t0, t2);
  let b2 = lerp(a2, a3, t1, t3);
  lerp(b1, b2, t1, t2)
}

/// The four control points of the spline segment from point `i` to
/// `i + 1`: past the ends, the end segment is mirrored.
fn controls(xy: &[[f32; 2]], i: usize) -> [[f32; 2]; 4] {
  let n = xy.len();
  let p1 = xy[i];
  let p2 = xy[i + 1];
  let p0 = if i > 0 {
    xy[i - 1]
  } else {
    [2.0 * p1[0] - p2[0], 2.0 * p1[1] - p2[1]]
  };
  let p3 = if i + 2 < n {
    xy[i + 2]
  } else {
    [2.0 * p2[0] - p1[0], 2.0 * p2[1] - p1[1]]
  };
  [p0, p1, p2, p3]
}

/// A point `t` of the way from `a` to `b`, with every field interpolated
/// and the position `at`. Falling only between two falling points.
pub fn blend(a: &ChannelPoint, b: &ChannelPoint, t: f32, at: [f32; 2]) -> ChannelPoint {
  let lerp = |u: f32, v: f32| u + (v - u) * t;
  ChannelPoint {
    x: at[0],
    y: at[1],
    level: lerp(a.level, b.level),
    bed: lerp(a.bed, b.bed),
    width: lerp(a.width, b.width),
    depth: lerp(a.depth, b.depth),
    discharge: lerp(a.discharge, b.discharge),
    slope: lerp(a.slope, b.slope),
    speed: lerp(a.speed, b.speed),
    curvature: lerp(a.curvature, b.curvature),
    celsius: lerp(a.celsius, b.celsius),
    rapids: lerp(a.rapids, b.rapids),
    falling: if t <= 0.0 {
      a.falling
    } else if t >= 1.0 {
      b.falling
    } else {
      a.falling && b.falling
    },
    order: a.order,
  }
}

/// Move each unpinned point across the flow onto the lowest ground within
/// [`THALWEG_REACH`] samples, with a penalty of 0.02 g a sample of offset
/// so flat ground keeps it near its cell.
fn thalweg(xy: &mut [[f32; 2]], pins: &[bool], map: &HeightMap, metres: f32) {
  let n = xy.len();
  let original = xy.to_vec();
  let steps = (THALWEG_REACH / THALWEG_STEP).round() as i32;

  for i in 1..n.saturating_sub(1) {
    if pins[i] {
      continue;
    }

    let (a, b) = (original[i - 1], original[i + 1]);
    let (tx, ty) = (b[0] - a[0], b[1] - a[1]);
    let length = length2(tx, ty);

    if length < 1e-6 {
      continue;
    }

    let normal = [-ty / length, tx / length];
    let p = original[i];
    let mut best = (height_at(map, p[0], p[1]), 0.0f32);

    // Nearest offsets first, so ties keep the point near its cell.
    for k in 1..=steps {
      for side in [-1.0f32, 1.0] {
        let offset = k as f32 * THALWEG_STEP * side;
        let (x, y) = (p[0] + normal[0] * offset, p[1] + normal[1] * offset);
        let cost = height_at(map, x, y) + 0.02 * metres * offset.abs();

        if cost < best.0 {
          best = (cost, offset);
        }
      }
    }

    xy[i] = [p[0] + normal[0] * best.1, p[1] + normal[1] * best.1];
  }
}

/// Turn a stream's cell chain into a smooth, evenly resampled centreline.
/// `pinned` marks the points that must not move, and `kept` those that
/// move with the curve but stay points of it (the ends of steps, where
/// falls are placed); both, and the two ends, are kept in order in the
/// result and marked as its pins. Every other point stays within
/// `max(0.75 samples, half the valley floor)` of its cell and on ground its
/// channel may climb (see [`ground_allows`]).
pub fn continuous_centreline(
  points: &[ChannelPoint],
  pinned: &[bool],
  kept: &[bool],
  map: &HeightMap,
  metres: f32,
) -> Smoothed {
  let n = points.len();

  if n < 3 {
    return Smoothed {
      points: points.to_vec(),
      pins: vec![true; n],
    };
  }

  let mut pins = pinned.to_vec();
  pins[0] = true;
  pins[n - 1] = true;
  let vertices: Vec<bool> = pins
    .iter()
    .zip(kept)
    .map(|(pin, kept)| *pin || *kept)
    .collect();
  let original: Vec<[f32; 2]> = points.iter().map(|p| [p.x, p.y]).collect();
  let floors = floor_widths(points, map, metres);
  let leash: Vec<f32> = floors
    .iter()
    .map(|floor| (0.5 * floor / metres).max(0.75))
    .collect();
  // Moves are measured from the water level, or from the ground of the
  // point's own cell where the channel will be cut below it: a point may
  // not climb higher than where it started.
  // So each point has a ceiling: the highest ground it may stand on (see
  // [`ground_allows`]).
  let top: Vec<f32> = points
    .iter()
    .zip(&original)
    .map(|(p, o)| {
      let raised = ChannelPoint {
        level: p.level.max(height_at(map, o[0], o[1])),
        ..*p
      };
      ceiling(&raised, metres)
    })
    .collect();
  let allowed = |i: usize, at: [f32; 2]| height_at(map, at[0], at[1]) <= top[i];
  let mut xy = original.clone();
  thalweg(&mut xy, &pins, map, metres);
  // Keep a move of point `i` from `from` to `to` in the valley: near its
  // cell, and never up the valley side (back towards `from`, in four
  // bisection steps, as far as it must).
  let settle = |i: usize, from: [f32; 2], to: [f32; 2]| {
    let o = original[i];
    let (dx, dy) = (to[0] - o[0], to[1] - o[1]);
    let away = length2(dx, dy);
    let to = if away > leash[i] {
      [o[0] + dx * leash[i] / away, o[1] + dy * leash[i] / away]
    } else {
      to
    };

    if allowed(i, to) {
      return to;
    }

    let place = |w: f32| {
      [
        from[0] + (to[0] - from[0]) * w,
        from[1] + (to[1] - from[1]) * w,
      ]
    };
    let (mut good, mut bad) = (0.0f32, 1.0f32);

    for _ in 0..4 {
      let mid = 0.5 * (good + bad);

      if allowed(i, place(mid)) {
        good = mid;
      } else {
        bad = mid;
      }
    }

    place(good)
  };

  for _ in 0..TAUBIN_ITERATIONS {
    let before = xy.clone();

    for factor in [TAUBIN_LAMBDA, TAUBIN_MU] {
      let last = xy.clone();

      for i in 1..n - 1 {
        if pins[i] {
          continue;
        }

        let (a, p, b) = (last[i - 1], last[i], last[i + 1]);
        xy[i] = [
          p[0] + factor * (0.5 * (a[0] + b[0]) - p[0]),
          p[1] + factor * (0.5 * (a[1] + b[1]) - p[1]),
        ];
      }
    }

    for i in 1..n - 1 {
      if !pins[i] {
        xy[i] = settle(i, before[i], xy[i]);
      }
    }
  }

  // Taubin smoothing keeps bends several samples long, as it should, but
  // also the corners where the valley turns within a sample or two. Ease
  // any point bending tighter than 1.5 max(w, g) towards its neighbours'
  // midpoint, a little at a time, within the same limits.
  let ease = |xy: &mut Vec<[f32; 2]>| {
    for _ in 0..BEND_PASSES {
      let mut eased = false;

      for i in 1..n - 1 {
        if pins[i] {
          continue;
        }

        let (a, p, b) = (xy[i - 1], xy[i], xy[i + 1]);
        let least = 1.5 * points[i].width.max(metres) / metres;

        if radius(a, p, b) >= least {
          continue;
        }

        let middle = [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])];
        let to = [
          p[0] + 0.5 * (middle[0] - p[0]),
          p[1] + 0.5 * (middle[1] - p[1]),
        ];
        xy[i] = settle(i, p, to);
        eased = true;
      }

      if !eased {
        break;
      }
    }
  };
  ease(&mut xy);

  // Where the spline between two points swings onto ground its channel
  // may not climb, draw both points halfway back to their cells, so the
  // curve yields smoothly instead of kinking at single points.
  for _ in 0..4 {
    let mut strained = false;

    for i in 0..n - 1 {
      let c = controls(&xy, i);
      let clear = (1..4).all(|k| {
        let t = k as f32 / 4.0;
        let at = catmull_rom(c, t);
        height_at(map, at[0], at[1]) <= top[i] + (top[i + 1] - top[i]) * t
      });

      if !clear {
        strained = true;

        for j in [i, i + 1] {
          if !pins[j] {
            xy[j] = [
              0.5 * (xy[j][0] + original[j][0]),
              0.5 * (xy[j][1] + original[j][1]),
            ];
          }
        }
      }
    }

    if !strained {
      break;
    }

    // Drawing points back can leave a corner of its own.
    ease(&mut xy);
  }

  resample(points, &xy, &vertices, metres)
}

/// Resample the spline through `xy` (one position per point of `points`)
/// at [`spacing`], span by span between pins, keeping every pin.
pub fn resample(points: &[ChannelPoint], xy: &[[f32; 2]], pins: &[bool], metres: f32) -> Smoothed {
  const SUBSTEPS: usize = 4;
  let n = points.len();
  let mut out = Smoothed {
    points: Vec::with_capacity(n * 2),
    pins: Vec::with_capacity(n * 2),
  };
  let mut first = points[0];
  first.x = xy[0][0];
  first.y = xy[0][1];
  out.points.push(first);
  out.pins.push(true);
  // Dense samples of one span: segment, parameter, position and spacing
  // units covered so far.
  let mut dense: Vec<(usize, f32, [f32; 2], f32)> = Vec::new();
  let mut start = 0;

  for end in 1..n {
    if !pins[end] {
      continue;
    }

    dense.clear();
    dense.push((start, 0.0, xy[start], 0.0));

    for i in start..end {
      let c = controls(xy, i);

      for k in 1..=SUBSTEPS {
        let t = k as f32 / SUBSTEPS as f32;
        dense.push((i, t, catmull_rom(c, t), 0.0));
      }
    }

    // Spacing units along the span: a point every `spacing`, or every 0.4
    // radius in a bend the valley holds tighter, so no point turns by more
    // than about 25 degrees.
    let mut units = 0.0f32;

    for k in 1..dense.len() {
      let (i, t, at, _) = dense[k];
      let previous = dense[k - 1].2;
      let step = length2(at[0] - previous[0], at[1] - previous[1]) * metres;
      let width = points[i].width + (points[i + 1].width - points[i].width) * t;
      let bend = radius(previous, at, dense[(k + 1).min(dense.len() - 1)].2) * metres;
      units += step / spacing(width, metres).min(0.4 * bend);
      dense[k].3 = units;
    }

    let count = units.round().max(1.0) as usize;
    let mut j = 0;

    for k in 1..count {
      let target = units * k as f32 / count as f32;

      while j + 1 < dense.len() && dense[j + 1].3 < target {
        j += 1;
      }

      let (a, b) = (dense[j], dense[(j + 1).min(dense.len() - 1)]);
      let w = ((target - a.3) / (b.3 - a.3).max(1e-6)).clamp(0.0, 1.0);
      let at = [
        a.2[0] + (b.2[0] - a.2[0]) * w,
        a.2[1] + (b.2[1] - a.2[1]) * w,
      ];
      // The spline parameter, carried across a segment boundary.
      let (segment, t) = if b.0 != a.0 {
        (b.0, b.1 * w)
      } else {
        (a.0, a.1 + (b.1 - a.1) * w)
      };
      let point = blend(&points[segment], &points[segment + 1], t, at);

      out.points.push(point);
      out.pins.push(false);
    }

    let mut pin = points[end];
    pin.x = xy[end][0];
    pin.y = xy[end][1];
    out.points.push(pin);
    out.pins.push(true);
    start = end;
  }

  // Interpolation can let the level rise over a bend in the profile's
  // smoothing; water never runs uphill.
  for i in 1..out.points.len() {
    let previous = out.points[i - 1];
    let point = &mut out.points[i];
    point.level = point.level.min(previous.level);
    point.bed = point.bed.min(previous.bed);
  }

  out
}

/// Douglas-Peucker simplification to `tolerance` samples, never dropping
/// a point `keep` marks, nor one whose water level is more than 2 cm off
/// the straight line between the points kept either side (the carve
/// interpolates levels along each segment).
pub fn simplify(points: &[ChannelPoint], keep: &[bool], tolerance: f32) -> Vec<ChannelPoint> {
  let n = points.len();

  if n <= 2 {
    return points.to_vec();
  }

  let mut kept = vec![false; n];
  kept[0] = true;
  kept[n - 1] = true;

  for (slot, keep) in kept.iter_mut().zip(keep) {
    *slot |= *keep;
  }

  let mut stack = Vec::new();
  let mut a = 0;

  for (b, &keep) in kept.iter().enumerate().take(n).skip(1) {
    if keep {
      stack.push((a, b));
      a = b;
    }
  }

  while let Some((a, b)) = stack.pop() {
    let (pa, pb) = (&points[a], &points[b]);
    let (dx, dy) = (pb.x - pa.x, pb.y - pa.y);
    let length = length2(dx, dy).max(1e-6);
    let mut worst = (0.0f32, 0);

    for (k, p) in points.iter().enumerate().take(b).skip(a + 1) {
      let along = (((p.x - pa.x) * dx + (p.y - pa.y) * dy) / (length * length)).clamp(0.0, 1.0);
      let level = pa.level + (pb.level - pa.level) * along;
      let off = (((p.x - pa.x) * dy - (p.y - pa.y) * dx).abs() / length / tolerance)
        .max((p.level - level).abs() / 0.02);

      if off > worst.0 {
        worst = (off, k);
      }
    }

    if worst.0 > 1.0 {
      kept[worst.1] = true;
      stack.push((a, worst.1));
      stack.push((worst.1, b));
    }
  }

  points
    .iter()
    .zip(&kept)
    .filter(|(_, k)| **k)
    .map(|(p, _)| *p)
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::terrain::channels::{
    condition_channels, raw_streams, CarveRecord, ChannelContext, Channels,
  };
  use crate::terrain::hydrology::build_hydrology;
  use vista_types::RiverOptions;

  /// Build and carve the rivers of `map`.
  fn channels(map: &mut HeightMap, options: &RiverOptions) -> Channels {
    let hydrology = build_hydrology(map, &[], options, 9);
    let streams = raw_streams(&hydrology);
    let mut record = CarveRecord::new(map.heights.len());
    let context = ChannelContext {
      surface: &[],
      options,
      seed: 9,
    };
    condition_channels(map, &hydrology, streams, &context, &mut record)
  }

  fn options() -> RiverOptions {
    RiverOptions {
      min_catchment_km2: 0.5,
      meanders: 0.0,
      inflow: vista_types::RiverInflows::Mode(vista_types::InflowMode::None),
      ..RiverOptions::default()
    }
  }

  /// A V valley 160 samples at 30 m, its axis through (10, 10) at 30
  /// degrees to the grid, falling along it to the map edge.
  fn slanted_valley() -> HeightMap {
    let (sin, cos) = 30f32.to_radians().sin_cos();
    crate::terrain::hydrology::tests::map_from(160, 30.0, move |x, y| {
      let (dx, dy) = (x as f32 - 10.0, y as f32 - 10.0);
      let along = dx * cos + dy * sin;
      let across = -dx * sin + dy * cos;
      400.0 - along * 1.2 + across.abs() * 6.0
    })
  }

  /// The longest reach.
  fn trunk(channels: &Channels) -> Vec<ChannelPoint> {
    channels
      .reaches
      .iter()
      .max_by_key(|reach| reach.points.len())
      .expect("a river")
      .points
      .clone()
  }

  #[test]
  fn a_valley_across_the_grid_gets_a_smooth_centreline_on_its_axis() {
    let mut map = slanted_valley();
    let points = trunk(&channels(&mut map, &options()));
    let (sin, cos) = 30f32.to_radians().sin_cos();
    let xy: Vec<[f32; 2]> = points.iter().map(|p| [p.x, p.y]).collect();
    // Away from the head and the map edge, where the points are pinned.
    let inner = |p: &[f32; 2]| p[0] > 3.0 && p[1] > 3.0 && p[0] < 156.0 && p[1] < 156.0;

    for p in xy.iter().filter(|p| inner(p)) {
      let across = -(p[0] - 10.0) * sin + (p[1] - 10.0) * cos;
      assert!(
        across.abs() <= 0.75,
        "{across} samples off the axis at {p:?}"
      );
    }

    let turns = crate::terrain::river_metrics::turn_angles(&xy).angles;
    // Points within a sample of the pinned head and mouth bend to meet them.
    let near_end = |p: &[f32; 2]| {
      let (first, last) = (xy[0], xy[xy.len() - 1]);
      length2(p[0] - first[0], p[1] - first[1]) < 1.5
        || length2(p[0] - last[0], p[1] - last[1]) < 1.5
    };

    for (k, turn) in turns.iter().enumerate() {
      if inner(&xy[k]) && inner(&xy[k + 2]) && !near_end(&xy[k + 1]) {
        assert!(*turn <= 5.0, "a {turn} degree turn at {:?}", xy[k + 1]);
      }
    }

    // Evenly spaced, except next to pins: the ends and the ends of steps
    // (whitewater).
    let metres = 30.0;
    let step = |k: usize| {
      points[k.saturating_sub(1)..(k + 3).min(points.len())]
        .iter()
        .any(|p| p.rapids > 0.0 || p.falling)
    };

    for (k, pair) in points
      .windows(2)
      .enumerate()
      .skip(2)
      .take(points.len().saturating_sub(5))
    {
      if step(k) {
        continue;
      }

      let gap = length2(pair[1].x - pair[0].x, pair[1].y - pair[0].y) * metres;
      let want = spacing(pair[0].width, metres);
      assert!((gap / want - 1.0).abs() <= 0.1, "{gap} m apart, not {want}");
    }

    // Levels and beds never rise.
    for pair in points.windows(2) {
      assert!(pair[1].level <= pair[0].level && pair[1].bed <= pair[0].bed);
    }
  }

  /// A straight run of points along x with pins at `pins`.
  fn run(n: usize, width: f32) -> Vec<ChannelPoint> {
    (0..n)
      .map(|i| ChannelPoint {
        x: 10.0 + i as f32,
        y: 20.0 + if i % 2 == 0 { 0.0 } else { 1.0 },
        level: 10.0 - i as f32 * 0.05,
        bed: 9.0 - i as f32 * 0.05,
        width,
        depth: 1.0,
        ..ChannelPoint::default()
      })
      .collect()
  }

  #[test]
  fn pins_stay_exactly_where_they_were() {
    let map = crate::terrain::hydrology::tests::map_from(64, 12.0, |_, _| 0.0);
    let points = run(30, 3.0);
    let mut pinned = vec![false; 30];
    pinned[11] = true;
    pinned[12] = true;
    let smoothed = continuous_centreline(&points, &pinned, &[false; 30], &map, 12.0);
    let kept: Vec<[f32; 2]> = smoothed
      .points
      .iter()
      .zip(&smoothed.pins)
      .filter(|(_, pin)| **pin)
      .map(|(p, _)| [p.x, p.y])
      .collect();
    let want: Vec<[f32; 2]> = [0, 11, 12, 29]
      .iter()
      .map(|i| [points[*i].x, points[*i].y])
      .collect();
    assert_eq!(kept, want);
    // The zigzag between pins is smoothed away.
    assert!(smoothed.points.iter().any(|p| p.y.fract() != 0.0));
  }

  #[test]
  fn smoothing_never_climbs_onto_a_spur() {
    // A tight bend round a spur: the valley runs east along y = 20, then
    // turns north up x = 30; the corner inside the bend (x > 30, y > 20)
    // is a spur 40 m high.
    let map = crate::terrain::hydrology::tests::map_from(64, 12.0, |x, y| {
      if x > 30 && y > 20 {
        40.0
      } else {
        0.0
      }
    });
    let mut points: Vec<ChannelPoint> = (10..=30)
      .map(|x| [x as f32, 20.0])
      .chain((21..=40).map(|y| [30.0, y as f32]))
      .enumerate()
      .map(|(i, [x, y])| ChannelPoint {
        x,
        y,
        level: 1.0 - i as f32 * 0.01,
        bed: 0.0,
        width: 3.0,
        depth: 1.0,
        ..ChannelPoint::default()
      })
      .collect();
    points.iter_mut().for_each(|p| p.bed = p.level - p.depth);
    let pinned = vec![false; points.len()];
    let smoothed = continuous_centreline(&points, &pinned, &pinned, &map, 12.0);

    for p in &smoothed.points {
      assert!(
        height_at(&map, p.x, p.y) <= p.level + p.depth.max(1.0) + 0.75 * p.slope * 12.0 + 1e-3,
        "on the spur at {}, {}",
        p.x,
        p.y
      );
    }
  }
}
