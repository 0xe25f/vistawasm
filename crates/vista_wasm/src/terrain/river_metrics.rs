//! Measures of river geometry, for tests and the geometry report: how far
//! drawn rivers are from grid routing (points on sample nodes, segments
//! along the eight grid directions, 45- and 90-degree turns, dead-straight
//! runs) and how natural they are (sinuosity, bend radius, junction
//! angles, Horton's bifurcation ratios).
//!
//! Points are in heightmap sample coordinates; lengths in metres.

use crate::maths::length2;
use crate::terrain::channels::{ChannelPoint, Reach};

/// Valley spacing (Perron, Kirchner and Dietrich, 2009), in samples: the
/// median distance from each point of a channel on a slope over 10 % to
/// the nearest point of another reach running within 30 degrees of it,
/// within 24 samples, braids aside. Channels a sample or two apart down a
/// planar slope are the furrows of a grid; real hillslopes keep valleys 6
/// to 12 apart. `None` when no point has such a neighbour.
pub fn valley_spacing(reaches: &[Reach]) -> Option<f32> {
  const REACH: f32 = 24.0;
  // Each point: its reach, position, unit direction and slope.
  let mut points: Vec<(usize, [f32; 2], [f32; 2], f32)> = Vec::new();

  // A braided belt's threads run side by side within it by nature.
  for (r, reach) in reaches
    .iter()
    .enumerate()
    .filter(|(_, reach)| reach.kind == crate::terrain::channels::ReachKind::Main)
  {
    let p = &reach.points;

    for k in 0..p.len() {
      let (a, b) = (&p[k.saturating_sub(1)], &p[(k + 1).min(p.len() - 1)]);
      let l = length2(b.x - a.x, b.y - a.y);

      if l > 1e-6 {
        points.push((
          r,
          [p[k].x, p[k].y],
          [(b.x - a.x) / l, (b.y - a.y) / l],
          p[k].slope,
        ));
      }
    }
  }

  let cell = |at: [f32; 2]| {
    (
      (at[0] / REACH).floor() as i32,
      (at[1] / REACH).floor() as i32,
    )
  };
  let mut grid: std::collections::HashMap<(i32, i32), Vec<usize>> =
    std::collections::HashMap::new();

  for (index, point) in points.iter().enumerate() {
    grid.entry(cell(point.1)).or_default().push(index);
  }

  let parallel = 30f32.to_radians().cos();
  let mut gaps = Vec::new();

  for (r, at, toward, slope) in &points {
    if *slope <= 0.1 {
      continue;
    }

    let (cx, cy) = cell(*at);
    let mut nearest = REACH;

    for gy in cy - 1..=cy + 1 {
      for gx in cx - 1..=cx + 1 {
        for &other in grid.get(&(gx, gy)).map_or(&[][..], Vec::as_slice) {
          let (q, there, way, _) = &points[other];

          if q != r && toward[0] * way[0] + toward[1] * way[1] >= parallel {
            nearest = nearest.min(length2(there[0] - at[0], there[1] - at[1]));
          }
        }
      }
    }

    if nearest < REACH {
      gaps.push(nearest);
    }
  }

  median(&mut gaps)
}

/// Path length over the straight distance between the ends; 1 for fewer
/// than two points or ends that meet.
pub fn sinuosity(points: &[[f32; 2]]) -> f32 {
  let n = points.len();

  if n < 2 {
    return 1.0;
  }

  let chord = length2(
    points[n - 1][0] - points[0][0],
    points[n - 1][1] - points[0][1],
  );
  let path = path_length(points, 1.0);

  if chord < 1e-6 {
    1.0
  } else {
    path / chord
  }
}

/// Length of a polyline in metres.
pub fn path_length(points: &[[f32; 2]], metres: f32) -> f32 {
  points
    .windows(2)
    .map(|p| length2(p[1][0] - p[0][0], p[1][1] - p[0][1]) * metres)
    .sum()
}

/// The share of segments within 1 degree of one of the eight grid
/// directions.
pub fn grid_aligned_fraction(points: &[[f32; 2]]) -> f32 {
  let mut aligned = 0usize;
  let mut count = 0usize;

  for pair in points.windows(2) {
    let (dx, dy) = (pair[1][0] - pair[0][0], pair[1][1] - pair[0][1]);

    if length2(dx, dy) < 1e-6 {
      continue;
    }

    let angle = dy.atan2(dx).to_degrees().rem_euclid(45.0);
    aligned += usize::from(angle.min(45.0 - angle) <= 1.0);
    count += 1;
  }

  aligned as f32 / count.max(1) as f32
}

/// The share of points exactly on a sample node.
pub fn on_nodes_fraction(points: &[[f32; 2]]) -> f32 {
  let on = points
    .iter()
    .filter(|p| p[0].fract() == 0.0 && p[1].fract() == 0.0)
    .count();
  on as f32 / points.len().max(1) as f32
}

/// Turning angles in degrees at each interior point (0 where two points
/// coincide), with the shares within 1 degree of 45 and of 90.
pub struct Turns {
  pub angles: Vec<f32>,
  pub share_45: f32,
  pub share_90: f32,
}

/// See [`Turns`].
pub fn turn_angles(points: &[[f32; 2]]) -> Turns {
  let angles: Vec<f32> = points
    .windows(3)
    .map(|p| {
      let u = [p[1][0] - p[0][0], p[1][1] - p[0][1]];
      let v = [p[2][0] - p[1][0], p[2][1] - p[1][1]];
      let (lu, lv) = (length2(u[0], u[1]), length2(v[0], v[1]));

      if lu < 1e-6 || lv < 1e-6 {
        return 0.0;
      }

      let cos = ((u[0] * v[0] + u[1] * v[1]) / (lu * lv)).clamp(-1.0, 1.0);
      cos.acos().to_degrees()
    })
    .collect();
  let share = |target: f32| {
    angles
      .iter()
      .filter(|a| (**a - target).abs() <= 1.0)
      .count() as f32
      / angles.len().max(1) as f32
  };

  Turns {
    share_45: share(45.0),
    share_90: share(90.0),
    angles,
  }
}

/// The longest run, in metres, of consecutive interior points turning by
/// less than half a degree: the segments either side of them.
pub fn longest_straight_run(points: &[[f32; 2]], metres: f32) -> f32 {
  let turns = turn_angles(points).angles;
  let segment = |i: usize| {
    length2(
      points[i + 1][0] - points[i][0],
      points[i + 1][1] - points[i][1],
    ) * metres
  };
  let mut best = 0.0f32;
  let mut run = 0.0f32;
  let mut open = false;

  for (k, turn) in turns.iter().enumerate() {
    // Interior point k + 1 joins segments k and k + 1.
    if *turn < 0.5 {
      if !open {
        run = segment(k);
        open = true;
      }

      run += segment(k + 1);
      best = best.max(run);
    } else {
      open = false;
    }
  }

  best
}

/// Radius of curvature, in metres, at each interior point: the circle
/// through it and its neighbours (infinite on a straight line).
pub fn radii(points: &[[f32; 2]], metres: f32) -> Vec<f32> {
  points
    .windows(3)
    .map(|p| {
      let a = length2(p[1][0] - p[0][0], p[1][1] - p[0][1]);
      let b = length2(p[2][0] - p[1][0], p[2][1] - p[1][1]);
      let c = length2(p[2][0] - p[0][0], p[2][1] - p[0][1]);
      let cross = ((p[1][0] - p[0][0]) * (p[2][1] - p[0][1])
        - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]))
        .abs();

      if cross < 1e-9 {
        f32::INFINITY
      } else {
        a * b * c / (2.0 * cross) * metres
      }
    })
    .collect()
}

/// The smallest radius of curvature over the width, at interior points.
pub fn min_radius_over_width(points: &[ChannelPoint], metres: f32) -> f32 {
  let xy = xy(points);
  radii(&xy, metres)
    .iter()
    .enumerate()
    .map(|(i, r)| r / points[i + 1].width.max(1e-3))
    .fold(f32::INFINITY, f32::min)
}

/// The points of a reach as sample coordinates.
pub fn xy(points: &[ChannelPoint]) -> Vec<[f32; 2]> {
  points.iter().map(|p| [p.x, p.y]).collect()
}

/// One tributary meeting another reach.
pub struct Junction {
  /// The angle between the tributary's last 2 w and the main reach's
  /// downstream tangent there, in degrees.
  pub angle: f32,
}

/// Every join: a reach whose last point lies within a tenth of a sample of
/// a point of another reach that is not that reach's first point (a reach
/// starting there is its distributary or continuation). The tributary's
/// direction is taken over its last 2 w; the main reach's tangent from
/// its neighbours either side.
pub fn junction_angles(reaches: &[Reach], metres: f32) -> Vec<Junction> {
  let mut junctions = Vec::new();

  for (index, reach) in reaches.iter().enumerate() {
    let points = &reach.points;

    let Some(last) = points.last() else {
      continue;
    };

    let mut found = None;

    for (main, other) in reaches.iter().enumerate() {
      if main == index {
        continue;
      }

      if let Some(at) = other
        .points
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, p)| length2(p.x - last.x, p.y - last.y) <= 0.1)
        .map(|(at, _)| at)
        .next()
      {
        found = Some((main, at));
        break;
      }
    }

    let Some((main, at)) = found else {
      continue;
    };

    // The tributary's direction over its last 2 w.
    let reach_back = 2.0 * last.width;
    let mut back = points.len() - 1;
    let mut walked = 0.0;

    while back > 0 && walked < reach_back {
      walked += length2(
        points[back].x - points[back - 1].x,
        points[back].y - points[back - 1].y,
      ) * metres;
      back -= 1;
    }

    let t = [last.x - points[back].x, last.y - points[back].y];
    let m = &reaches[main].points;
    let (before, after) = (&m[at.saturating_sub(1)], &m[(at + 1).min(m.len() - 1)]);
    let d = [after.x - before.x, after.y - before.y];
    let (lt, ld) = (length2(t[0], t[1]), length2(d[0], d[1]));

    if lt < 1e-6 || ld < 1e-6 {
      continue;
    }

    let cos = ((t[0] * d[0] + t[1] * d[1]) / (lt * ld)).clamp(-1.0, 1.0);
    junctions.push(Junction {
      angle: cos.acos().to_degrees(),
    });
  }

  junctions
}

/// Horton's bifurcation ratios `N(k) / N(k + 1)` for Strahler orders 1
/// upwards, from the order of each Strahler stream.
pub fn bifurcation_ratios(orders: &[u8]) -> Vec<f32> {
  let top = orders.iter().copied().max().unwrap_or(0) as usize;
  let mut counts = vec![0usize; top + 1];

  for order in orders {
    counts[*order as usize] += 1;
  }

  (1..top)
    .map(|k| counts[k] as f32 / counts[k + 1].max(1) as f32)
    .collect()
}

/// The order of every Strahler stream: each run of one order along a
/// reach (a main stem carries on through confluences, gaining order).
pub fn strahler_streams(reaches: &[Reach]) -> Vec<u8> {
  let mut orders = Vec::new();

  for reach in reaches {
    let mut last = 0;

    for p in &reach.points {
      if p.order != last && p.order > 0 {
        orders.push(p.order);
        last = p.order;
      }
    }
  }

  orders
}

/// The median of `values`, or `None` when empty.
pub fn median(values: &mut [f32]) -> Option<f32> {
  if values.is_empty() {
    return None;
  }

  values.sort_by(f32::total_cmp);
  Some(values[values.len() / 2])
}

/// A reach's valley slope: the level it falls over the straight distance
/// between its ends.
pub fn valley_slope(points: &[ChannelPoint], metres: f32) -> f32 {
  let (first, last) = (&points[0], &points[points.len() - 1]);
  let chord = length2(last.x - first.x, last.y - first.y) * metres;
  (first.level - last.level) / chord.max(1e-3)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn grid_paths_measure_as_grid_paths() {
    // A staircase: east, north-east, east, north-east.
    let stairs = [[0.0, 0.0], [1.0, 0.0], [2.0, 1.0], [3.0, 1.0], [4.0, 2.0]];
    assert_eq!(on_nodes_fraction(&stairs), 1.0);
    assert_eq!(grid_aligned_fraction(&stairs), 1.0);
    let turns = turn_angles(&stairs);
    assert_eq!(turns.share_45, 1.0);
    assert_eq!(turns.share_90, 0.0);

    // A circle of radius 10 samples is on no node, nor grid-aligned.
    let circle: Vec<[f32; 2]> = (0..40)
      .map(|i| {
        let a = i as f32 * 0.1 + 0.05;
        [10.0 * a.cos() + 0.37, 10.0 * a.sin() + 0.21]
      })
      .collect();
    assert!(on_nodes_fraction(&circle) == 0.0);
    assert!(grid_aligned_fraction(&circle) < 0.2);
    assert!(radii(&circle, 2.0).iter().all(|r| (r - 20.0).abs() < 0.1));
    assert!(longest_straight_run(&circle, 2.0) == 0.0);

    let line = [[0.0, 0.0], [1.0, 0.5], [2.0, 1.0], [3.0, 1.5], [4.0, 3.0]];
    let run = longest_straight_run(&line, 10.0);
    assert!((run - 3.0 * 1.25f32.sqrt() * 10.0).abs() < 1e-3, "{run}");
    assert!((sinuosity(&[[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]]) - 2f32.sqrt()).abs() < 1e-5);
  }

  #[test]
  fn horton_ratios_count_streams_by_order() {
    // Nine first-order streams, three second, one third.
    let mut orders = vec![1u8; 9];
    orders.extend([2, 2, 2, 3]);
    assert_eq!(bifurcation_ratios(&orders), vec![3.0, 3.0]);
  }

  /// One of the maps the geometry report measures.
  pub(crate) struct ReportMap {
    pub name: &'static str,
    pub landform: vista_types::LandformKind,
    pub metres: f32,
    pub edges: vista_types::TerrainEdges,
  }

  /// The four maps the river geometry report measures.
  pub(crate) fn report_maps() -> [ReportMap; 4] {
    use vista_types::{LandformKind, TerrainEdges};
    [
      ReportMap {
        name: "continental 12 m coast",
        landform: LandformKind::Continental,
        metres: 12.0,
        edges: TerrainEdges::Coast,
      },
      ReportMap {
        name: "rolling hills 30 m coast",
        landform: LandformKind::RollingHills,
        metres: 30.0,
        edges: TerrainEdges::Coast,
      },
      ReportMap {
        name: "continental 30 m open",
        landform: LandformKind::Continental,
        metres: 30.0,
        edges: TerrainEdges::Open,
      },
      ReportMap {
        name: "alpine 12 m coast",
        landform: LandformKind::Alpine,
        metres: 12.0,
        edges: TerrainEdges::Coast,
      },
    ]
  }

  /// Generate a report map at `size` samples a side as the demo's default
  /// scene does (seed 12345, ridged noise, island 0.35), bake its surface
  /// and build its rivers with `options`, as the engine does. Returns the
  /// carved map, the network and the build time in milliseconds.
  pub(crate) fn build_report_map(
    map: &ReportMap,
    size: u32,
    options: &vista_types::RiverOptions,
  ) -> (
    crate::terrain::HeightMap,
    crate::render::water::RiverNetwork,
    f64,
  ) {
    let terrain = vista_types::FractalTerrainOptions {
      seed: 12345,
      size,
      horizontal_scale_metres: map.metres,
      vertical_scale: 1.0,
      base_height_metres: Some(0.0),
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
      landform: map.landform,
      edges: map.edges,
    };
    let mut heights = crate::terrain::generate_fractal_heightmap(&terrain).unwrap();
    let (_, surface) = crate::render::terrain_mesh::bake_terrain_shading(
      &heights,
      &vista_types::BiomeOptions::default(),
      None,
    );
    let seed = crate::engine::terrain_seed(&heights);
    let record = crate::terrain::channels::CarveRecord::new(heights.heights.len());
    let started = std::time::Instant::now();
    let network = crate::render::water::build_river_network(
      &mut heights,
      options,
      crate::render::water::RiverSources {
        surface: &surface,
        seed,
        painted: Vec::new(),
        record,
      },
    );
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    (heights, network, elapsed)
  }

  /// Every measure of this module over one built map, as one line.
  pub(crate) fn report_line(
    name: &str,
    map: &crate::terrain::HeightMap,
    network: &crate::render::water::RiverNetwork,
    elapsed: f64,
  ) -> String {
    use crate::render::water::WATER_KIND_RIVER;
    let metres = map.metadata.metres_per_sample;
    let reaches = &network.reaches;
    let all: Vec<[f32; 2]> = reaches.iter().flat_map(|r| xy(&r.points)).collect();
    let mut on_nodes = 0.0;
    let mut aligned = 0.0;
    let (mut t45, mut t90, mut sharp, mut turns) = (0.0, 0.0, 0usize, 0usize);
    let mut straight = 0.0f32;
    let mut min_rw = f32::INFINITY;
    let (mut tight, mut interior) = (0usize, 0usize);
    let (mut flat, mut steep) = (Vec::new(), Vec::new());
    let mut widest = 0.0f32;

    for reach in reaches {
      let points = xy(&reach.points);
      let n = points.len() as f32;
      on_nodes += on_nodes_fraction(&points) * n;
      aligned += grid_aligned_fraction(&points) * (n - 1.0).max(0.0);
      let t = turn_angles(&points);
      t45 += t.share_45 * t.angles.len() as f32;
      t90 += t.share_90 * t.angles.len() as f32;
      turns += t.angles.len();

      for (k, angle) in t.angles.iter().enumerate() {
        let (a, b) = (&reach.points[k], &reach.points[k + 2]);
        sharp += usize::from(*angle > 30.0 && !(a.falling || b.falling));
      }

      straight = straight.max(longest_straight_run(&points, metres));

      if reach.points.len() >= 3 {
        min_rw = min_rw.min(min_radius_over_width(&reach.points, metres));

        for (k, r) in radii(&points, metres).iter().enumerate() {
          let w = reach.points[k + 1].width;
          tight += usize::from(*r < (1.5 * w).max(metres));
          interior += 1;
        }
      }

      widest = reach.points.iter().map(|p| p.width).fold(widest, f32::max);

      if path_length(&points, metres) > 300.0 {
        let slope = valley_slope(&reach.points, metres);

        if slope < 0.01 {
          flat.push(sinuosity(&points));
        } else if slope > 0.04 {
          steep.push(sinuosity(&points));
        }
      }
    }

    let mut angles: Vec<f32> = junction_angles(reaches, metres)
      .iter()
      .map(|j| j.angle)
      .collect();
    let format = |value: Option<f32>| value.map_or("none".to_string(), |v| format!("{v:.2}"));
    let ribbon = network
      .vertices
      .iter()
      .filter(|v| v.kind() == WATER_KIND_RIVER)
      .count();
    let joins = angles.len();
    let (lowest, highest) = (
      angles.iter().copied().fold(f32::INFINITY, f32::min),
      angles.iter().copied().fold(0.0f32, f32::max),
    );

    format!(
      "{name}: build {elapsed:.0} ms, {} reaches, {} points, on nodes {:.1} %, grid-aligned {:.1} %, \
        turns 45 {:.1} % / 90 {:.1} %, turns over 30 {sharp}, longest straight {straight:.0} m, \
        min r/w {min_rw:.2}, r under max(1.5 w, g) {:.2} %, joins {joins} at {lowest:.0} to {highest:.0} \
        (median {}), sinuosity flat {} ({}) / steep {} ({}), widest {widest:.2} m, \
        ribbon vertices {ribbon}, bank vertices {}, oxbow and lake vertices {}, inflows {:?}, \
        widest river's mouth at world {:?}, Horton's ratios {:?}, field {} tiles ({} KB), \
        valley spacing {} samples",
      reaches.len(),
      all.len(),
      100.0 * on_nodes / all.len().max(1) as f32,
      100.0 * aligned / all.len().saturating_sub(reaches.len()).max(1) as f32,
      100.0 * t45 / turns.max(1) as f32,
      100.0 * t90 / turns.max(1) as f32,
      100.0 * tight as f32 / interior.max(1) as f32,
      format(median(&mut angles)),
      format(median(&mut flat)),
      flat.len(),
      format(median(&mut steep)),
      steep.len(),
      network.bank_vertices.len(),
      network.vertices.len() - ribbon,
      network
        .inflows
        .iter()
        .map(|i| (i.position, i.discharge))
        .collect::<Vec<_>>(),
      reaches
        .iter()
        .max_by(|a, b| {
          let widest = |r: &Reach| r.points.iter().fold(0.0f32, |m, p| m.max(p.width));
          widest(a).total_cmp(&widest(b))
        })
        .and_then(|r| r.points.last())
        .map(|p| {
          let half = |size: u32| (size as f32 - 1.0) * 0.5 * metres;
          [
            p.x * metres - half(map.metadata.width),
            p.y * metres - half(map.metadata.height),
          ]
        }),
      bifurcation_ratios(&strahler_streams(reaches)),
      network.field.tile_count(),
      network.field.bytes() / 1024,
      format(valley_spacing(reaches)),
    )
  }

  /// The four diagnosis maps at 256 x 256 (same landforms and spacings)
  /// carry no grid artefacts: few points on sample nodes or segments along
  /// grid directions, no sharp turns except at falls and beside a stream's
  /// head, bends no tighter than the channel allows, no dead-straight run
  /// over 25 samples (only a stream down a planar slope or a straight
  /// gully of the terrain runs that far), and steep streams that follow
  /// their valleys nearly straight.
  #[test]
  fn rivers_carry_no_grid_artefacts() {
    for map in report_maps() {
      let (heights, network, _) =
        build_report_map(&map, 256, &vista_types::RiverOptions::default());
      let metres = heights.metadata.metres_per_sample;
      let name = map.name;
      let reaches = &network.reaches;
      let all: Vec<[f32; 2]> = reaches.iter().flat_map(|r| xy(&r.points)).collect();
      let segments = all.len() - reaches.len();
      let aligned: f32 = reaches
        .iter()
        .map(|r| grid_aligned_fraction(&xy(&r.points)) * (r.points.len() - 1) as f32)
        .sum();
      let (mut tight, mut interior) = (0usize, 0usize);
      let mut steep = Vec::new();

      // Heads and mouths are cells by definition; of the rest, at most 5 %
      // sit on sample nodes.
      let inner: Vec<[f32; 2]> = reaches
        .iter()
        .flat_map(|r| xy(&r.points[1..r.points.len() - 1]))
        .collect();
      let on_nodes = on_nodes_fraction(&inner);
      assert!(on_nodes <= 0.05, "{name}: {on_nodes} on nodes");
      assert!(
        aligned / segments as f32 <= 0.25,
        "{name}: {} along the grid",
        aligned / segments as f32
      );

      for reach in reaches {
        let points = xy(&reach.points);
        let head = points[0];

        for (k, turn) in turn_angles(&points).angles.iter().enumerate() {
          let (a, p, b) = (&reach.points[k], &reach.points[k + 1], &reach.points[k + 2]);
          let at_fall = [a, p, b].iter().any(|q| q.falling || q.rapids > 0.0);
          let by_head = length2(p.x - head[0], p.y - head[1]) < 1.0;
          assert!(
            *turn <= 30.0 || at_fall || by_head,
            "{name}: a {turn} degree turn at {}, {} (point {k} of {}, ends {:?} {:?})",
            p.x,
            p.y,
            points.len(),
            points[0],
            points[points.len() - 1]
          );
        }

        for (k, r) in radii(&points, metres).iter().enumerate() {
          let p = &reach.points[k + 1];
          let fall = reach.points[k..k + 3]
            .iter()
            .any(|q| q.falling || q.rapids > 0.0);
          let by_head = length2(p.x - head[0], p.y - head[1]) < 1.0;

          if !(fall || by_head) {
            assert!(
              *r >= p.width.max(0.5 * metres) - 1e-3,
              "{name}: a bend {r} m across at {}, {}, {} m wide",
              p.x,
              p.y,
              p.width
            );
          }

          tight += usize::from(*r < (1.5 * p.width).max(metres));
          interior += 1;
        }

        let run = longest_straight_run(&points, metres);
        assert!(run <= 25.0 * metres, "{name}: a straight run of {run} m");

        if path_length(&points, metres) > 300.0 && valley_slope(&reach.points, metres) > 0.04 {
          steep.push(sinuosity(&points));
        }
      }

      assert!(
        tight as f32 <= 0.01 * interior as f32,
        "{name}: {tight} of {interior} bends tight"
      );
      let steep = median(&mut steep).unwrap_or(1.0);
      assert!(steep <= 1.2, "{name}: steep streams' sinuosity {steep}");
    }
  }

  /// River water lies on the ground: on every diagnosis map, no river
  /// vertex above sea level is more than 3 m over the carved ground under
  /// it. A ribbon drawn between the samples a steep stream was routed
  /// through would otherwise hang in the air above the slope.
  #[test]
  fn river_water_never_hangs_above_the_ground() {
    for map in report_maps() {
      let (heights, network, _) =
        build_report_map(&map, 256, &vista_types::RiverOptions::default());
      let metres = heights.metadata.metres_per_sample;
      let half = [
        (heights.metadata.width as f32 - 1.0) * metres * 0.5,
        (heights.metadata.height as f32 - 1.0) * metres * 0.5,
      ];

      for vertex in &network.vertices {
        if vertex.kind() != crate::render::water::WATER_KIND_RIVER {
          continue;
        }

        let x = (vertex.position[0] + half[0]) / metres;
        let y = (vertex.position[2] + half[1]) / metres;
        let ground = crate::render::terrain_mesh::full_detail_height(&heights, x, y);

        if ground > heights.metadata.sea_level_metres {
          let above = vertex.position[1] - ground;
          assert!(
            above <= 3.0,
            "{}: water {above} m above the ground at {x}, {y}",
            map.name
          );
        }
      }
    }
  }

  #[test]
  fn valley_spacing_measures_the_gap_between_parallel_channels() {
    // Five channels 3 samples apart down a 15 % slope, and a cross channel
    // that is no one's neighbour.
    let channel = |x: f32, slope: f32, across: bool| Reach {
      points: (0..40)
        .map(|i| ChannelPoint {
          x: if across { i as f32 } else { x },
          y: if across { x } else { i as f32 },
          slope,
          ..ChannelPoint::default()
        })
        .collect(),
      ..Reach::default()
    };
    let mut reaches: Vec<Reach> = (0..5)
      .map(|k| channel(10.0 + 3.0 * k as f32, 0.15, false))
      .collect();
    reaches.push(channel(100.0, 0.15, true));
    assert_eq!(valley_spacing(&reaches), Some(3.0));
    // Gentle ground does not count, nor do channels too far apart.
    let gentle: Vec<Reach> = (0..5)
      .map(|k| channel(3.0 * k as f32, 0.05, false))
      .collect();
    assert_eq!(valley_spacing(&gentle), None);
    let apart: Vec<Reach> = (0..3)
      .map(|k| channel(30.0 * k as f32, 0.15, false))
      .collect();
    assert_eq!(valley_spacing(&apart), None);
  }

  /// Streams branch as drainage networks do: on the rolling-hills map,
  /// at the report's 512 x 512, Horton's bifurcation ratio for Strahler
  /// orders 1 to 3 is 2.5 to 6. A smaller copy has too few second-order
  /// streams for a steady ratio.
  #[test]
  fn rolling_hills_branch_with_hortons_ratios() {
    let (_, network, _) = build_report_map(
      &report_maps()[1],
      512,
      &vista_types::RiverOptions::default(),
    );
    let ratios = bifurcation_ratios(&strahler_streams(&network.reaches));
    assert!(ratios.len() >= 2, "{ratios:?}");

    for ratio in &ratios[..2] {
      assert!((2.5..=6.0).contains(ratio), "{ratios:?}");
    }
  }

  /// The river geometry report over its four maps at
  /// 512 x 512. Run with
  /// `cargo test -q -p vista_wasm river_geometry_report -- --ignored --nocapture`.
  #[test]
  #[ignore]
  fn river_geometry_report() {
    for map in report_maps() {
      let (heights, network, elapsed) =
        build_report_map(&map, 512, &vista_types::RiverOptions::default());
      println!("{}", report_line(map.name, &heights, &network, elapsed));
    }
  }
}
