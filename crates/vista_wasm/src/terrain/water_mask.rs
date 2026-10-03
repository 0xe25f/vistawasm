//! Painted water: a host's mask of rivers and lakes, carved and drawn like
//! the terrain's own.
//!
//! Lakes are flattened into basins below their rim, and above the sea, so
//! the drainage fills them to their lowest rim point and they overflow
//! there into the natural network. Painted water whose rim is at or near
//! the sea is part of the sea instead: it shelves out from the land like a
//! coast. Rivers are thinned to centrelines, stripped of the spurs thinning
//! leaves, traced end to end, turned to run downhill (tributaries into the
//! river they touch), and handed to the channel stage, which cuts them at
//! their painted width (or wider, where their discharge asks for it) and
//! as deep as a natural river of that width, and joins them to the natural
//! network. Every change is recorded, so clearing the mask restores the
//! terrain exactly.

use vista_types::WaterMask;

use crate::errors::VistaResult;
use crate::maths::smoothstep;
use crate::terrain::channels::CarveRecord;
use crate::terrain::drainage::neighbours;
use crate::terrain::HeightMap;

/// Mask values from this up are lakes; below it, from 1, rivers.
pub const LAKE: u8 = 128;

/// The longest spur, in samples, that thinning a stroke can leave: half
/// the widest brush.
const MAX_SPUR: usize = 128;

/// The shallowest painted lake, in metres: water whose rim is closer to
/// the sea than this is sea water.
const MIN_LAKE_DEPTH: f32 = 1.5;

/// A painted lake's bed stays this far above the sea, in metres.
const BED_ABOVE_SEA: f32 = 0.5;

/// How fast painted sea water deepens away from the land: 1 in 50, a
/// gently shelving coast.
const SHELF_GRADE: f32 = 0.02;

/// A painted river centreline, from its upstream end.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintedRiver {
  /// Heightmap sample coordinates.
  pub points: Vec<[f32; 2]>,
  /// Painted width in metres.
  pub width: f32,
  /// Whether its last point lies on a painted river listed before it,
  /// which it joins there.
  pub joins: bool,
}

/// Check a mask's size and data.
pub fn validate(mask: &WaterMask) -> VistaResult<()> {
  crate::terrain::painted::check_size("setWaterMask", mask.width, mask.height, mask.data.len())
}

/// Resample a mask to `width x height` samples: the nearest value decides
/// between no water, river and lake, and river strength is interpolated
/// bilinearly between river samples.
pub fn resample(mask: &WaterMask, width: u32, height: u32) -> Vec<u8> {
  let at = |x: u32, y: u32| mask.data[(y * mask.width + x) as usize];
  let strength = |x: u32, y: u32| {
    let value = at(x, y);
    if value < LAKE {
      value as f32
    } else {
      0.0
    }
  };
  let mut out = Vec::with_capacity(width as usize * height as usize);

  for y in 0..height {
    for x in 0..width {
      let fx = x as f32 * (mask.width - 1) as f32 / (width - 1).max(1) as f32;
      let fy = y as f32 * (mask.height - 1) as f32 / (height - 1).max(1) as f32;
      let nearest = at(fx.round() as u32, fy.round() as u32);

      if nearest == 0 || nearest >= LAKE {
        out.push(nearest);
        continue;
      }

      let (x0, y0) = (fx as u32, fy as u32);
      let (x1, y1) = ((x0 + 1).min(mask.width - 1), (y0 + 1).min(mask.height - 1));
      let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
      let top = strength(x0, y0) + (strength(x1, y0) - strength(x0, y0)) * tx;
      let bottom = strength(x0, y1) + (strength(x1, y1) - strength(x0, y1)) * tx;
      out.push((top + (bottom - top) * ty).round().clamp(1.0, 127.0) as u8);
    }
  }

  out
}

/// Flatten every painted lake into a basin below its lowest rim point,
/// recording the changes, and return the painted rivers. `painted` is a
/// resampled mask of the map's size.
pub fn apply(map: &mut HeightMap, painted: &[u8], record: &mut CarveRecord) -> Vec<PaintedRiver> {
  let (width, height) = (map.metadata.width, map.metadata.height);
  let metres = map.metadata.metres_per_sample.max(0.001);
  let count = map.heights.len();
  let mut region = vec![u32::MAX; count];
  let mut slot_of = vec![u32::MAX; count];
  let mut stack = Vec::new();

  for start in 0..count as u32 {
    if painted[start as usize] < LAKE || region[start as usize] != u32::MAX {
      continue;
    }

    // One connected lake, and the lowest ground around it.
    region[start as usize] = start;
    stack.push(start);
    let mut cells = Vec::new();
    let mut rim = f32::MAX;

    while let Some(cell) = stack.pop() {
      cells.push(cell);

      for n in neighbours(width, height, cell) {
        let i = n as usize;

        if painted[i] >= LAKE {
          if region[i] == u32::MAX {
            region[i] = start;
            stack.push(n);
          }
        } else if !map.no_data[i] {
          rim = rim.min(map.heights[i]);
        }
      }
    }

    if rim == f32::MAX {
      continue;
    }

    for (slot, cell) in cells.iter().enumerate() {
      slot_of[*cell as usize] = slot as u32;
    }

    let sea = map.metadata.sea_level_metres;
    let depth = (0.05 * (cells.len() as f32 * metres * metres).sqrt()).max(MIN_LAKE_DEPTH);
    // Water that could not stand a basin's depth above the sea is sea
    // water, level with it.
    let joins_sea = rim < sea + MIN_LAKE_DEPTH;
    // Steps from the shore over the lake; sea water counts them from the
    // land only.
    let mut from_shore = vec![u32::MAX; cells.len()];
    let mut queue: Vec<usize> = Vec::new();

    for (slot, cell) in cells.iter().enumerate() {
      if neighbours(width, height, *cell)
        .any(|n| region[n as usize] != start && (!joins_sea || map.heights[n as usize] > sea))
      {
        from_shore[slot] = 0;
        queue.push(slot);
      }
    }

    let mut head = 0;

    while head < queue.len() {
      let slot = queue[head];
      head += 1;

      for n in neighbours(width, height, cells[slot]) {
        if region[n as usize] == start {
          let other = slot_of[n as usize] as usize;

          if from_shore[other] == u32::MAX {
            from_shore[other] = from_shore[slot] + 1;
            queue.push(other);
          }
        }
      }
    }

    // Sea water is never deeper than the sea beside it.
    let floor = if rim < sea { sea - rim } else { depth };

    for (slot, cell) in cells.iter().enumerate() {
      let i = *cell as usize;
      let steps = from_shore[slot].min(u16::MAX as u32) as f32;

      if !map.no_data[i] {
        let target = if joins_sea {
          // It shelves gently out from the land, as a coast does, so it
          // joins the sea with no edge and no deeper, darker patch.
          sea - (SHELF_GRADE * (steps + 0.5) * metres).min(depth).min(floor)
        } else {
          // The bed deepens away from the shore over three samples, to the
          // basin depth below the rim, but stays above the sea: where it
          // dipped under, the sea would show through the lake.
          let deep = 0.3 + 0.7 * smoothstep(steps / 3.0);
          (rim - depth * deep).max(sea + BED_ABOVE_SEA)
        };
        record.lower(map, i, target);
      }
    }
  }

  rivers(map, painted)
}

/// Thin the painted river strokes to centrelines one sample wide
/// (Zhang and Suen, 1984), then trace them.
fn rivers(map: &HeightMap, painted: &[u8]) -> Vec<PaintedRiver> {
  let (width, height) = (map.metadata.width as i32, map.metadata.height as i32);
  let mut on: Vec<bool> = painted.iter().map(|v| *v > 0 && *v < LAKE).collect();
  let at = |on: &[bool], x: i32, y: i32| {
    x >= 0 && y >= 0 && x < width && y < height && on[(y * width + x) as usize]
  };

  // Thin only within the strokes' bounds.
  let (mut x_min, mut y_min, mut x_max, mut y_max) = (width, height, -1, -1);

  for (index, value) in on.iter().enumerate() {
    if *value {
      let (x, y) = (index as i32 % width, index as i32 / width);
      (x_min, y_min, x_max, y_max) = (x_min.min(x), y_min.min(y), x_max.max(x), y_max.max(y));
    }
  }

  loop {
    let mut changed = false;

    for pass in 0..2 {
      let mut remove = Vec::new();

      for y in y_min..=y_max {
        for x in x_min..=x_max {
          if !on[(y * width + x) as usize] {
            continue;
          }

          let p = around(&on, width, height, x, y);
          let b = p.iter().filter(|v| **v).count();
          let a = runs(p);
          let (first, second) = if pass == 0 {
            (p[0] && p[2] && p[4], p[2] && p[4] && p[6])
          } else {
            (p[0] && p[2] && p[6], p[0] && p[4] && p[6])
          };

          if (2..=6).contains(&b) && a == 1 && !first && !second {
            remove.push((y * width + x) as usize);
          }
        }
      }

      changed |= !remove.is_empty();

      for index in remove {
        on[index] = false;
      }
    }

    if !changed {
      break;
    }
  }

  let runs_at = |on: &[bool], x: i32, y: i32| runs(around(on, width, height, x, y));
  // Sideways steps before diagonal ones, so a staircase is walked pixel by
  // pixel and none is left behind.
  const STEPS: [usize; 8] = [0, 2, 4, 6, 1, 3, 5, 7];
  // `owner` holds the line each pixel was traced into, or `WALKED` while
  // a spur is walked.
  const FREE: u32 = u32::MAX;
  const WALKED: u32 = u32::MAX - 1;
  let mut owner = vec![FREE; on.len()];
  let next = |on: &[bool], owner: &[u32], x: i32, y: i32| {
    STEPS
      .iter()
      .map(|k| (x + RING[*k].0, y + RING[*k].1))
      .find(|&(nx, ny)| at(on, nx, ny) && owner[(ny * width + nx) as usize] == FREE)
  };
  let starts: Vec<(i32, i32)> = (y_min..=y_max)
    .flat_map(|y| (x_min..=x_max).map(move |x| (x, y)))
    .filter(|&(x, y)| at(&on, x, y))
    .collect();

  // Thinning leaves short spurs from the centreline out to the edge of the
  // stroke where it bends or ends unevenly. A branch from an end to a fork
  // no longer than the stroke is wide at the fork is one of them.
  for &(x0, y0) in &starts {
    if !at(&on, x0, y0) || runs_at(&on, x0, y0) != 1 {
      continue;
    }

    let mut spur = vec![(y0 * width + x0) as usize];
    owner[spur[0]] = WALKED;
    let (mut x, mut y) = (x0, y0);
    let mut cut = false;

    while spur.len() <= MAX_SPUR {
      let Some((nx, ny)) = next(&on, &owner, x, y) else {
        break;
      };

      if runs_at(&on, nx, ny) >= 3 {
        cut = spur.len() <= half_width(painted, width, height, nx, ny) + 1;
        break;
      }

      (x, y) = (nx, ny);
      spur.push((y * width + x) as usize);
      owner[(y * width + x) as usize] = WALKED;
    }

    for index in spur {
      owner[index] = FREE;
      on[index] &= !cut;
    }
  }

  // Trace from every end, then around any closed loop left over.
  let mut lines: Vec<Vec<(i32, i32)>> = Vec::new();
  let starts: Vec<(i32, i32)> = starts.into_iter().filter(|&(x, y)| at(&on, x, y)).collect();
  let ends = starts.iter().filter(|&&(x, y)| runs_at(&on, x, y) <= 1);
  let others = starts.iter().filter(|&&(x, y)| runs_at(&on, x, y) > 1);

  for &(x0, y0) in ends.chain(others) {
    if owner[(y0 * width + x0) as usize] != FREE {
      continue;
    }

    let id = lines.len() as u32;
    let mut line = vec![(x0, y0)];
    owner[(y0 * width + x0) as usize] = id;
    let (mut x, mut y) = (x0, y0);

    while let Some((nx, ny)) = next(&on, &owner, x, y) {
      (x, y) = (nx, ny);
      owner[(y * width + x) as usize] = id;
      line.push((x, y));
    }

    lines.push(line);
  }

  // Where an end of a line stops beside another line, it joins it there.
  let join_at = |line: usize, (x, y): (i32, i32)| {
    RING
      .iter()
      .map(|(dx, dy)| (x + dx, y + dy))
      .find(|&(nx, ny)| at(&on, nx, ny) && owner[(ny * width + nx) as usize] != line as u32)
  };
  // Rivers before the tributaries that join them, so each is cut first
  // and its tributaries meet it at its level.
  let mut rivers: Vec<PaintedRiver> = Vec::new();
  let mut tributaries: Vec<PaintedRiver> = Vec::new();

  for (id, mut line) in lines.into_iter().enumerate() {
    if line.len() < 2 {
      continue;
    }

    let strength = line
      .iter()
      .map(|&(x, y)| painted[(y * width + x) as usize] as f32)
      .fold(1.0, f32::max);
    let (a, b) = (line[0], line[line.len() - 1]);
    let (join_a, join_b) = (join_at(id, a), join_at(id, b));
    let first = map.heights[(a.1 * width + a.0) as usize];
    let last = map.heights[(b.1 * width + b.0) as usize];
    let downhill = match (join_a, join_b) {
      // A tributary runs into the line it joins.
      (Some(_), None) => false,
      (None, Some(_)) => true,
      _ if (first - last).abs() > 1e-3 => first > last,
      // Level ends: flow towards the nearer sea or lake.
      _ => towards_water(map, painted, a) > towards_water(map, painted, b),
    };

    if !downhill {
      line.reverse();
    }

    let join = if downhill { join_b } else { join_a };
    line.extend(join);
    let river = PaintedRiver {
      points: line.iter().map(|&(x, y)| [x as f32, y as f32]).collect(),
      width: 1.0 + (strength - 1.0) / 126.0 * 59.0,
      joins: join.is_some(),
    };

    if river.joins {
      tributaries.push(river);
    } else {
      rivers.push(river);
    }
  }

  rivers.extend(tributaries);
  rivers
}

/// Neighbours clockwise from north: P2 to P9.
const RING: [(i32, i32); 8] = [
  (0, -1),
  (1, -1),
  (1, 0),
  (1, 1),
  (0, 1),
  (-1, 1),
  (-1, 0),
  (-1, -1),
];

/// Which of the eight neighbours of `(x, y)` are on, clockwise from
/// north. Kept out of line: it is used in several loops, and a copy in
/// each would only grow the WASM.
#[inline(never)]
fn around(on: &[bool], width: i32, height: i32, x: i32, y: i32) -> [bool; 8] {
  RING.map(|(dx, dy)| {
    let (nx, ny) = (x + dx, y + dy);
    nx >= 0 && ny >= 0 && nx < width && ny < height && on[(ny * width + nx) as usize]
  })
}

/// The separate runs of centreline among a pixel's neighbours: one at an
/// end, two along a line, three or more where it branches. Counting runs
/// rather than neighbours keeps the corners of a diagonal staircase,
/// which touch three pixels, on the line.
fn runs(p: [bool; 8]) -> usize {
  (0..8).filter(|i| !p[*i] && p[(i + 1) % 8]).count()
}

/// Half the width of a painted river stroke at `(x, y)`: the distance in
/// samples, ring by ring, to the nearest sample outside it.
fn half_width(painted: &[u8], width: i32, height: i32, x: i32, y: i32) -> usize {
  let outside = |(x, y): (i32, i32)| {
    x < 0 || y < 0 || x >= width || y >= height || {
      let value = painted[(y * width + x) as usize];
      value == 0 || value >= LAKE
    }
  };

  (1..MAX_SPUR as i32)
    .find(|&r| {
      (-r..=r).any(|d| {
        [
          (x + d, y - r),
          (x + d, y + r),
          (x - r, y + d),
          (x + r, y + d),
        ]
        .into_iter()
        .any(outside)
      })
    })
    .unwrap_or(MAX_SPUR as i32) as usize
}

/// Distance in samples from `from` to the nearest sea or painted lake,
/// searched outwards ring by ring.
fn towards_water(map: &HeightMap, painted: &[u8], from: (i32, i32)) -> i32 {
  let (width, height) = (map.metadata.width as i32, map.metadata.height as i32);
  let sea = map.metadata.sea_level_metres;
  let water = |x: i32, y: i32| {
    x >= 0 && y >= 0 && x < width && y < height && {
      let i = (y * width + x) as usize;
      painted[i] >= LAKE || map.heights[i] <= sea
    }
  };

  for r in 0..width.max(height) {
    for d in -r..=r {
      if water(from.0 + d, from.1 - r)
        || water(from.0 + d, from.1 + r)
        || water(from.0 - r, from.1 + d)
        || water(from.0 + r, from.1 + d)
      {
        return r;
      }
    }
  }

  i32::MAX
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::render::water::{build_river_network, RiverNetwork, RiverSources};
  use crate::terrain::channels::{channel_depth, width_discharge};
  use vista_types::{RiverOptions, TerrainMetadata};

  const SIZE: u32 = 128;

  /// A plain rising 0.8 m a sample to the east from 6 m below the sea at
  /// its west edge, 40 m a sample: the coast runs north to south at x =
  /// 7.5.
  fn slope() -> HeightMap {
    let metadata = TerrainMetadata {
      metres_per_sample: 40.0,
      sea_level_metres: 0.0,
      ..Default::default()
    };
    let heights = (0..SIZE * SIZE)
      .map(|i| (i % SIZE) as f32 * 0.8 - 6.0 + ((i / SIZE) as f32 * 0.3).sin() * 0.2)
      .collect();
    let no_data = vec![false; (SIZE * SIZE) as usize];
    HeightMap::from_values(SIZE, SIZE, heights, no_data, metadata).unwrap()
  }

  fn paint(value: impl Fn(f32, f32) -> u8) -> Vec<u8> {
    (0..SIZE * SIZE)
      .map(|i| value((i % SIZE) as f32, (i / SIZE) as f32))
      .collect()
  }

  /// A brush stroke `radius` samples either side of a gentle S curve from
  /// x = 20 to 110, as the demo's river brush paints one.
  fn stroke(radius: f32, value: u8) -> impl Fn(f32, f32) -> u8 {
    move |x, y| {
      let centre = 64.0 + 10.0 * (x / 18.0).sin();
      u8::from((20.0..110.0).contains(&x) && (y - centre).abs() <= radius) * value
    }
  }

  /// Apply `painted` to `map` and build its water with natural rivers off.
  fn build(map: &mut HeightMap, painted: &[u8]) -> (Vec<PaintedRiver>, RiverNetwork) {
    let mut record = CarveRecord::new(map.heights.len());
    let rivers = apply(map, painted, &mut record);
    let options = RiverOptions {
      enabled: false,
      ..Default::default()
    };
    let sources = RiverSources {
      surface: &[],
      seed: 1,
      painted: rivers.clone(),
      record,
    };
    let network = build_river_network(map, &options, sources);
    (rivers, network)
  }

  #[test]
  fn a_thick_painted_stroke_becomes_one_river_as_deep_as_a_natural_one_of_its_width() {
    let mut map = slope();
    let (rivers, network) = build(&mut map, &paint(stroke(3.5, 127)));

    // Thinning leaves spurs and staircase corners; neither splits it.
    assert_eq!(rivers.len(), 1, "{rivers:?}");
    assert!(rivers[0].points.len() > 85);
    assert!(rivers[0].points[0][0] > 100.0, "it runs west, downhill");
    assert_eq!(network.reaches.len(), 1);

    // A natural river 60 m wide carries about 490 m³/s and is about 4 m
    // deep; the painted one is cut to the same depth.
    let natural = channel_depth(width_discharge(60.0, 1.0));
    assert!(natural > 4.0, "{natural}");

    for point in &network.reaches[0].points {
      assert!(point.width >= 59.9, "width {}", point.width);
      assert!(
        (point.depth - natural).abs() < 0.01,
        "depth {} not {natural}",
        point.depth
      );
    }

    // Its bed is carved into the plain.
    let middle = network.reaches[0].points[network.reaches[0].points.len() / 2];
    let index = (middle.y.round() as u32 * SIZE + middle.x.round() as u32) as usize;
    assert!(map.heights[index] < middle.level - 0.5 * natural);
  }

  #[test]
  fn a_thin_diagonal_stroke_becomes_one_river() {
    // As the import check paints one: a staircase a sample or two wide.
    let mut map = slope();
    let painted = paint(|x, y| {
      u8::from((y - 40.0 - (x - 30.0) * 0.3).abs() < 1.2 && x > 30.0 && x < 110.0) * 70
    });
    let (rivers, network) = build(&mut map, &painted);

    assert_eq!(rivers.len(), 1, "{rivers:?}");
    assert_eq!(network.reaches.len(), 1);
    assert!(network.reaches[0].points.iter().all(|p| p.depth > 2.0));
  }

  #[test]
  fn a_painted_tributary_joins_its_river_and_ends_there() {
    let mut map = slope();
    let main = stroke(2.0, 127);
    // A branch from the north-east meeting the main stroke at x = 70.
    let painted = paint(|x, y| {
      let branch = (70.0..100.0).contains(&x) && (y - (60.0 - (x - 70.0))).abs() <= 1.5;
      main(x, y).max(u8::from(branch) * 60)
    });
    let (rivers, network) = build(&mut map, &painted);

    assert_eq!(rivers.len(), 2, "{rivers:?}");
    assert!(!rivers[0].joins);
    assert!(rivers[1].joins, "the branch joins the main river");
    let end = rivers[1].points[rivers[1].points.len() - 1];
    assert!(
      rivers[0].points.contains(&end),
      "the tributary ends on the river, at {end:?}"
    );
    assert_eq!(network.reaches.len(), 2);
  }

  #[test]
  fn a_painted_lake_near_the_sea_keeps_its_bed_above_the_sea() {
    let mut map = slope();
    let before = map.heights.clone();
    // A lake about 640 m across whose rim, at x = 21, is 11 m up: a basin
    // for its size would reach 17 m below the sea.
    let painted = paint(|x, y| u8::from((x - 30.0).hypot(y - 64.0) < 8.0) * 200);
    let (_, network) = build(&mut map, &painted);

    assert_eq!(network.lakes.len(), 1);
    let rim = before[(64 * SIZE + 21) as usize];
    assert!((network.lakes[0].surface - rim).abs() < 1.0);

    for (index, height) in map.heights.iter().enumerate() {
      if painted[index] > 0 {
        assert!(
          *height >= BED_ABOVE_SEA - 1e-4,
          "sample {index} dug to {height}"
        );
      }
    }
  }

  #[test]
  fn painted_water_across_the_coast_joins_the_sea_with_no_edge_or_deeper_patch() {
    let mut map = slope();
    let before = map.heights.clone();
    let painted = paint(|x, y| u8::from((x - 12.0).hypot(y - 64.0) < 7.0) * 200);
    let (_, network) = build(&mut map, &painted);
    let metres = map.metadata.metres_per_sample;
    let beside = |index: usize, test: &dyn Fn(usize) -> bool| {
      neighbours(SIZE, SIZE, index as u32).any(|n| test(n as usize))
    };
    // The lowest ground beside the painted water, which is sea floor.
    let rim = (0..map.heights.len())
      .filter(|i| painted[*i] == 0 && beside(*i, &|n| painted[n] > 0))
      .map(|i| before[i])
      .fold(f32::MAX, f32::min);
    assert!(rim < 0.0);

    // No lake surface: it is the sea.
    assert!(network.lakes.is_empty());

    for (index, value) in painted.iter().enumerate() {
      let height = map.heights[index];

      if *value == 0 {
        assert_eq!(height, before[index]);
        continue;
      }

      // Under water, and never deeper than it was or than the sea beside it.
      assert!(height <= 0.0, "sample {index} stands at {height}");
      assert!(
        height >= before[index].min(rim) - 1e-4,
        "sample {index} dug to {height}"
      );

      // It shelves out from the land: half a sample's grade at the shore.
      if before[index] > 0.0 && beside(index, &|n| painted[n] == 0 && before[n] > 0.0) {
        assert!(
          height >= -SHELF_GRADE * metres * 0.5 - 1e-4,
          "shore sample {index} at {height}"
        );
      }
    }
  }
}
