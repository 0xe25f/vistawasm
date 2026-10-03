//! Meanders that migrate.
//!
//! A reduced-complexity centreline model (Howard and Knutson, 1984, built
//! on the bend theory of Ikeda, Parker and Sawai, 1981): each point of a
//! river's smooth centreline moves sideways at a rate set by the curvature
//! there and, with a weight that decays downstream, by the curvature
//! upstream. The fast core of the flow is thrown against the outer bank
//! after each bend's apex, so bends grow, skew downstream and migrate;
//! loops tighten until their necks meet, are cut off and leave oxbow lakes.
//! Confinement by the valley walls, uneven bank strength and the channel
//! pattern (meandering gives way to braiding on steep ground) set where
//! and how fast it happens. Everything runs once per river build, on the
//! CPU, from seeded values, so a seed always gives the same rivers.

use crate::maths::{hash_u64, length2, smoothstep, value_noise};
use crate::terrain::centreline::{blend, spacing};
use crate::terrain::channels::{height_at, manning_speed, rock_banks, ChannelPoint};
use crate::terrain::heightmap::HeightMap;

/// Weight of the local curvature in the migration rate (Howard and
/// Knutson's Omega).
const OMEGA: f32 = -1.0;

/// Weight of the upstream curvature (Howard and Knutson's Gamma).
const GAMMA: f32 = 2.5;

/// Friction factor: the upstream influence decays over `L = d / (2 Cf)`.
/// With the weights above, bends of wavelength about `8.2 L` grow
/// fastest; 0.03 puts that at 9 to 14 widths for this crate's hydraulic
/// geometry (w / d from 10 to 15), the wavelength of real meanders.
const FRICTION: f32 = 0.03;

/// The furthest the fastest point moves in one step, in widths.
const STEP_WIDTHS: f32 = 0.08;

/// Rates are scaled by the fastest movement, `E |R1|`, but never by less
/// than this. The curvature of a nearly straight reach is noise, and
/// moving it at full speed makes the points oscillate. The local term
/// smooths like curve shortening, so the explicit step is stable while
/// `0.08 E / LEAST_FASTEST` stays under `0.5 (1.5)^2` for the 1.5 w
/// curvature stencil: E reaches 1.3, so 0.12 keeps it under 0.9 of the
/// 1.125 limit. Set on the synthetic floodplain fixture (see
/// `channels::tests::floodplain`), where it gives a sinuosity near 1.5
/// at the default strength and half maturity.
const LEAST_FASTEST: f32 = 0.12;

/// Neck cut-offs are looked for every this many steps.
const CUTOFF_EVERY: usize = 4;

/// Two points at least this many widths apart along the river, and
/// closer than [`NECK_WIDTHS`] in space, cut off the loop between them.
const LOOP_WIDTHS: f32 = 6.0;

/// See [`LOOP_WIDTHS`].
const NECK_WIDTHS: f32 = 1.2;

/// Most loops a reach keeps as oxbows (the youngest).
const MAX_OXBOWS: usize = 12;

/// The braiding threshold slope `a Q^-0.44` (Leopold and Wolman, 1957;
/// Parker, 1976): `a` in SI units.
const BRAIDING_SLOPE: f32 = 0.02;

/// The slope above which a channel carrying `discharge` m³/s braids,
/// where it has the room.
pub fn braiding_threshold(discharge: f32) -> f32 {
  BRAIDING_SLOPE * discharge.max(0.01).powf(-0.44)
}

/// The meandering share of the channel pattern at a point: 1 on gentle
/// ground, falling to 0 as the slope nears the braiding threshold.
pub fn meandering(point: &ChannelPoint) -> f32 {
  let threshold = braiding_threshold(point.discharge);
  1.0 - smoothstep((point.slope - 0.5 * threshold) / (0.5 * threshold))
}

/// Room to meander: 0 where the valley floor is narrower than 2 w, 1
/// where it is wider than 6 w.
pub fn confinement(floor: f32, width: f32) -> f32 {
  smoothstep((floor - 2.0 * width) / (4.0 * width))
}

/// Whether a channel `width` metres wide can show its meanders on a grid
/// `metres` apart: its wavelength, about 11 w, spans at least 3 samples.
pub fn representable(width: f32, metres: f32) -> bool {
  11.0 * width >= 3.0 * metres
}

/// Settings for one reach's migration.
pub struct Migration<'a> {
  /// The ground the river runs over.
  pub map: &'a HeightMap,
  /// Heightmap sample spacing in metres.
  pub metres: f32,
  /// How strongly bends migrate, 0 to 1.
  pub strength: f32,
  /// How long they have migrated, 0 to 1.
  pub maturity: f32,
  /// Seeds the start offsets and bank strength.
  pub seed: u64,
  /// Whether the reach ends in the sea, where migration tapers out over
  /// the last 2 w instead of half a wavelength.
  pub sea_mouth: bool,
}

/// A loop cut off by a neck cut-off.
#[derive(Clone, Debug, PartialEq)]
pub struct CutLoop {
  /// The loop's centreline, from the upstream neck to the downstream one.
  pub points: Vec<ChannelPoint>,
  /// Where the loop left the channel, on the channel after the cut.
  pub connection: [f32; 2],
  /// How old it is: `(N - step cut) / N`, near 1 for a loop cut early in
  /// the run and near 0 for one cut at its end.
  pub age: f32,
}

/// A migrated reach.
#[derive(Clone, Debug, Default)]
pub struct Migrated {
  /// The new centreline.
  pub points: Vec<ChannelPoint>,
  /// Loops cut off, oldest first.
  pub cut: Vec<CutLoop>,
  /// Earlier centrelines, every eighth of the run, for the floodplain's
  /// scroll bars.
  pub snapshots: Vec<Vec<ChannelPoint>>,
}

/// A point of the migrating centreline.
#[derive(Clone, Copy)]
struct Node {
  point: ChannelPoint,
  /// Never moves.
  pin: bool,
  /// How strongly it migrates, `E`.
  strength: f32,
}

/// Arc length in metres at each point.
fn arc<'a>(points: impl Iterator<Item = &'a ChannelPoint>, metres: f32) -> Vec<f32> {
  let mut total = 0.0;
  let mut last: Option<&ChannelPoint> = None;
  points
    .map(|p| {
      if let Some(q) = last {
        total += length2(p.x - q.x, p.y - q.y) * metres;
      }

      last = Some(p);
      total
    })
    .collect()
}

/// Whether ground at `x, y` is low enough for `point`'s channel: no higher
/// than `max(1.5 d, 1 m)` above its water, the valley walls' foot.
fn confined(map: &HeightMap, point: &ChannelPoint, x: f32, y: f32) -> bool {
  height_at(map, x, y) <= point.level + (1.5 * point.depth).max(1.0)
}

/// Migrate one reach's centreline. `pins` marks points that never move
/// (heads, mouths, step ends and the map edge), and `floors` the valley
/// floor width at each point. Returns the reach unmoved where nothing can
/// migrate.
pub fn migrate(
  points: &[ChannelPoint],
  pins: &[bool],
  floors: &[f32],
  settings: &Migration<'_>,
) -> Migrated {
  let metres = settings.metres;
  let map = settings.map;
  let n = points.len();
  let s = arc(points.iter(), metres);
  // E = meanders x pattern x confinement x bank, tapered to nothing over
  // half a wavelength around every pin, and over 2 w at a sea mouth.
  let noise_seed = hash_u64(settings.seed ^ 0x006d_6561_6e64_6572);
  let mut nodes: Vec<Node> = (0..n)
    .map(|i| {
      let p = &points[i];
      let mut strength = 0.0;

      if representable(p.width, metres) && !p.falling {
        // Banks of uneven strength, fixed in space, 4 w across, break the
        // symmetry of the loops.
        let cell = 4.0 * p.width / metres;
        let bank =
          (1.0 - rock_banks(p)) * (1.0 + 0.3 * value_noise(noise_seed, p.x / cell, p.y / cell));
        strength = settings.strength * meandering(p) * confinement(floors[i], p.width) * bank;

        for j in (0..n).filter(|j| pins[*j]) {
          let w = points[j].width;
          let reach = if j == n - 1 && settings.sea_mouth {
            2.0
          } else {
            5.5
          } * w;
          strength *= smoothstep((s[i] - s[j]).abs() / reach.max(1e-3));
        }
      }

      Node {
        point: *p,
        pin: pins[i],
        strength,
      }
    })
    .collect();

  if n < 8 || nodes.iter().all(|node| node.strength <= 1e-4) {
    return Migrated {
      points: points.to_vec(),
      ..Migrated::default()
    };
  }

  let steps = (40.0 + 360.0 * settings.maturity.clamp(0.0, 1.0)).round() as usize;
  let cap = 1.0 + 1.6 * settings.strength;
  let mut cut = Vec::new();
  let mut snapshots = Vec::new();
  let (right, bottom) = (
    (map.metadata.width - 1) as f32,
    (map.metadata.height - 1) as f32,
  );
  // A straight smooth reach has no curvature to grow from: seed the bends
  // with a sum of three sines of 8 to 16 w, 0.05 w high.
  let unit = |k: u64| (hash_u64(settings.seed ^ k) >> 40) as f32 / (1u64 << 24) as f32;
  let tau = std::f32::consts::TAU;
  let seeded = |i: usize, p: &ChannelPoint| {
    (1..4u64)
      .map(|k| (s[i] / ((8.0 + 8.0 * unit(2 * k)) * p.width) * tau + tau * unit(2 * k + 1)).sin())
      .sum::<f32>()
      * 0.05
      / 3.0
      * p.width
      * (nodes[i].strength / settings.strength).min(1.0)
      / metres
  };
  let seeds: Vec<f32> = (0..n).map(|i| seeded(i, &points[i])).collect();

  for step in 0..=steps {
    let count = nodes.len();
    let s = arc(nodes.iter().map(|node| &node.point), metres);
    let (first, last) = (nodes[0].point, nodes[count - 1].point);

    if s[count - 1] > cap * length2(last.x - first.x, last.y - first.y) * metres {
      break;
    }

    // Unit left normals, and curvatures in 1/m from the circle through
    // each point and the points about 1.5 widths either side, then a
    // 1-2-1 filter. The noise between neighbouring points, much closer
    // than a width on wide rivers, never drives the migration, and the
    // wide stencil lets the explicit step stay stable at a useful size
    // (see [`LEAST_FASTEST`]). A circle through three points of a bend is
    // exact however far apart they are, so tight loops are measured well.
    let mut normal = vec![[0.0f32; 2]; count];
    let mut raw = vec![0.0f32; count];

    for i in 0..count {
      let at = |k: usize| &nodes[k.min(count - 1)].point;
      let (a, b) = (at(i.saturating_sub(1)), at(i + 1));
      let length = length2(b.x - a.x, b.y - a.y).max(1e-6);
      normal[i] = [-(b.y - a.y) / length, (b.x - a.x) / length];
      let w = at(i).width;
      let m = (1.5 * w / spacing(w, metres)).round().max(1.0) as usize;
      let (a, b, c) = (at(i.saturating_sub(m)), at(i), at(i + m));
      let u = [(b.x - a.x) * metres, (b.y - a.y) * metres];
      let v = [(c.x - b.x) * metres, (c.y - b.y) * metres];
      let product = length2(u[0], u[1]) * length2(v[0], v[1]) * length2(u[0] + v[0], u[1] + v[1]);

      if product > 1e-9 {
        raw[i] = 2.0 * (u[0] * v[1] - u[1] * v[0]) / product;
      }
    }

    if step == 0 {
      // The seeded bends, before the first step.
      for (i, node) in nodes.iter_mut().enumerate() {
        let p = node.point;
        let (x, y) = (p.x + normal[i][0] * seeds[i], p.y + normal[i][1] * seeds[i]);

        if node.strength > 0.0 && confined(map, &p, x, y) {
          node.point.x = x;
          node.point.y = y;
        }
      }

      continue;
    }

    // The upstream sums of `R0 G` and `G`: G is exponential, so both are
    // running totals carried down node by node.
    let (mut weighted, mut weights, mut nominal) = (0.0f32, 0.0f32, 0.0f32);
    let mut rate = vec![0.0f32; count];
    // The fastest point that can move, so that tapered points near pins,
    // where curvature gathers but nothing moves, never slow the rest.
    let mut fastest = LEAST_FASTEST;

    for k in 0..count {
      let node = &nodes[k];
      let kappa = 0.25 * (raw[k.saturating_sub(1)] + 2.0 * raw[k] + raw[(k + 1).min(count - 1)]);

      if k > 0 {
        let decay = (-2.0 * FRICTION * (s[k] - s[k - 1]) / node.point.depth.max(0.1)).exp();
        weighted = (weighted + nominal) * decay;
        weights = (weights + 1.0) * decay;
      }

      nominal = node.point.width * kappa;
      let upstream = if weights > 0.0 {
        weighted / weights
      } else {
        0.0
      };
      rate[k] = OMEGA * nominal + GAMMA * upstream;
      fastest = fastest.max((rate[k] * node.strength).abs());
    }

    for (k, node) in nodes.iter_mut().enumerate() {
      if node.pin || node.strength <= 0.0 {
        continue;
      }

      // Positive rates move towards the outer bank: right of a left turn.
      let p = node.point;
      let shift = STEP_WIDTHS * p.width * node.strength * rate[k] / fastest / metres;

      // Valley walls deflect bends: the move stops at the last half step
      // that stays off them.
      for fraction in [1.0, 0.5, 0.25] {
        let x = (p.x - normal[k][0] * shift * fraction).clamp(0.0, right);
        let y = (p.y - normal[k][1] * shift * fraction).clamp(0.0, bottom);

        if confined(map, &p, x, y) {
          node.point.x = x;
          node.point.y = y;
          break;
        }
      }
    }

    respace(&mut nodes, metres);

    if step % CUTOFF_EVERY == 0 {
      cut_necks(
        &mut nodes,
        metres,
        (steps + 1 - step) as f32 / steps as f32,
        &mut cut,
      );
    }

    if step % (steps / 8).max(1) == 0 && snapshots.len() < 8 {
      snapshots.push(nodes.iter().map(|node| node.point).collect());
    }
  }

  if cut.len() > MAX_OXBOWS {
    cut.drain(..cut.len() - MAX_OXBOWS);
  }

  // Water never runs uphill, and a longer path over the same fall is
  // gentler: the channel slope is the valley's over the sinuosity.
  let mut points: Vec<ChannelPoint> = nodes.iter().map(|node| node.point).collect();

  for i in 1..points.len() {
    let previous = points[i - 1];
    let point = &mut points[i];
    point.level = point.level.min(previous.level);
    point.bed = point.bed.min(previous.bed);
  }

  let s = arc(points.iter(), metres);
  let count = points.len();
  let slopes: Vec<f32> = (0..count)
    .map(|i| {
      let reach = 3.0 * points[i].width;
      let a = (0..i).rev().find(|j| s[i] - s[*j] >= reach).unwrap_or(0);
      let b = (i + 1..count)
        .find(|j| s[*j] - s[i] >= reach)
        .unwrap_or(count - 1);
      (points[a].level - points[b].level) / (s[b] - s[a]).max(1e-3)
    })
    .collect();

  for (point, slope) in points.iter_mut().zip(slopes) {
    if !(point.falling || point.rapids > 0.0) {
      point.slope = slope.max(0.0);
      point.speed = manning_speed(point.depth, point.slope);
    }
  }

  Migrated {
    points,
    cut,
    snapshots,
  }
}

/// Keep points between half and one and a half times their spacing
/// apart: a point is added in a gap that grew too long, and one of two
/// that came too close is dropped (never a pin).
fn respace(nodes: &mut Vec<Node>, metres: f32) {
  let mut i = 0;

  while i + 1 < nodes.len() {
    let (a, b) = (nodes[i], nodes[i + 1]);
    let (p, q) = (&a.point, &b.point);
    let want = spacing(p.width, metres) / metres;
    let gap = length2(q.x - p.x, q.y - p.y);

    if gap > 1.5 * want {
      let node = Node {
        point: blend(p, q, 0.5, [0.5 * (p.x + q.x), 0.5 * (p.y + q.y)]),
        pin: false,
        strength: 0.5 * (a.strength + b.strength),
      };
      nodes.insert(i + 1, node);
      continue;
    }

    if gap < 0.5 * want && nodes.len() > 3 {
      let drop = if !b.pin && i + 2 < nodes.len() {
        i + 1
      } else if !a.pin && i > 0 {
        i
      } else {
        usize::MAX
      };

      if drop != usize::MAX {
        nodes.remove(drop);
        continue;
      }
    }

    i += 1;
  }
}

/// Cut off every loop whose neck has closed: two points at least
/// [`LOOP_WIDTHS`] apart along the river and closer than [`NECK_WIDTHS`]
/// widths in space, found with a uniform grid of 2 w cells. The loop
/// between them is removed and the join smoothed with three Taubin passes
/// over five points either side.
fn cut_necks(nodes: &mut Vec<Node>, metres: f32, age: f32, cut: &mut Vec<CutLoop>) {
  // Each cut removes at least one point, so there are fewer cuts than
  // points; the bound stops widths that are not numbers.
  for _ in 0..nodes.len() {
    let s = arc(nodes.iter().map(|node| &node.point), metres);
    let count = nodes.len();
    let widest = nodes.iter().fold(0.0f32, |m, node| m.max(node.point.width));
    let cell = (2.0 * widest / metres).max(0.5);
    let (mut low, mut high) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);

    for node in nodes.iter() {
      let p = &node.point;
      low = [low[0].min(p.x), low[1].min(p.y)];
      high = [high[0].max(p.x), high[1].max(p.y)];
    }

    let columns = ((high[0] - low[0]) / cell) as usize + 1;
    let rows = ((high[1] - low[1]) / cell) as usize + 1;
    let slot = |p: &ChannelPoint| {
      (
        ((p.x - low[0]) / cell) as usize,
        ((p.y - low[1]) / cell) as usize,
      )
    };
    // Points bucketed by cell: counts, then offsets, then indices.
    let mut start = vec![0u32; columns * rows + 1];

    for node in nodes.iter() {
      let (cx, cy) = slot(&node.point);
      start[cy * columns + cx + 1] += 1;
    }

    for k in 0..columns * rows {
      start[k + 1] += start[k];
    }

    let mut next = start.clone();
    let mut bucket = vec![0u32; count];

    for (i, node) in nodes.iter().enumerate() {
      let (cx, cy) = slot(&node.point);
      let at = &mut next[cy * columns + cx];
      bucket[*at as usize] = i as u32;
      *at += 1;
    }

    // The first neck along the river, and its nearest partner downstream.
    let mut found: Option<(usize, usize)> = None;

    for i in 0..count {
      let p = nodes[i].point;
      let (cx, cy) = slot(&p);

      for gy in cy.saturating_sub(1)..=(cy + 1).min(rows - 1) {
        for gx in cx.saturating_sub(1)..=(cx + 1).min(columns - 1) {
          let slot = gy * columns + gx;

          for &j in &bucket[start[slot] as usize..start[slot + 1] as usize] {
            let j = j as usize;
            let q = &nodes[j].point;

            if j > i
              && s[j] - s[i] >= LOOP_WIDTHS * p.width
              && found.is_none_or(|(_, b)| j < b)
              && length2(q.x - p.x, q.y - p.y) * metres < NECK_WIDTHS * p.width
              && !nodes[i + 1..j].iter().any(|node| node.pin)
            {
              found = Some((i, j));
            }
          }
        }
      }

      if found.is_some() {
        break;
      }
    }

    let Some((i, j)) = found else {
      return;
    };

    let points = nodes[i..=j].iter().map(|node| node.point).collect();
    nodes.drain(i + 1..j);

    for _ in 0..3 {
      for factor in [0.5f32, -0.53] {
        let last: Vec<[f32; 2]> = nodes
          .iter()
          .map(|node| [node.point.x, node.point.y])
          .collect();

        for k in i.saturating_sub(4).max(1)..(i + 6).min(nodes.len() - 1) {
          if !nodes[k].pin {
            let (a, p, b) = (last[k - 1], last[k], last[k + 1]);
            nodes[k].point.x = p[0] + factor * (0.5 * (a[0] + b[0]) - p[0]);
            nodes[k].point.y = p[1] + factor * (0.5 * (a[1] + b[1]) - p[1]);
          }
        }
      }
    }

    cut.push(CutLoop {
      points,
      connection: [nodes[i].point.x, nodes[i].point.y],
      age,
    });
  }
}

/// Whether a centreline crosses itself: two segments that share no point
/// intersect.
#[cfg(test)]
pub fn crosses_itself(points: &[ChannelPoint]) -> bool {
  let cross = |o: &ChannelPoint, a: &ChannelPoint, b: &ChannelPoint| {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
  };
  let n = points.len();

  for i in 0..n.saturating_sub(1) {
    for j in i + 2..n - 1 {
      let (a, b, c, d) = (&points[i], &points[i + 1], &points[j], &points[j + 1]);

      if cross(c, d, a) * cross(c, d, b) < 0.0 && cross(a, b, c) * cross(a, b, d) < 0.0 {
        return true;
      }
    }
  }

  false
}
