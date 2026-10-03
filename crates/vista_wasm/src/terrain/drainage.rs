//! Drainage on a height grid: depression filling, flow receivers, stack
//! order, and upstream accumulation.
//!
//! The river extraction in `render/water.rs`, the stream-power solver in
//! `terrain/stream_power.rs`, and the terrain tests all route water the
//! same way, so the routing lives here once.

use std::collections::BinaryHeap;

/// Marks a cell that drains nowhere (an outlet, or not yet routed).
pub const NO_RECEIVER: u32 = u32::MAX;

/// Eight-way neighbour offsets: the four edges first, then the diagonals.
pub const OFFSETS: [(i32, i32); 8] = [
  (-1, 0),
  (1, 0),
  (0, -1),
  (0, 1),
  (-1, -1),
  (1, -1),
  (-1, 1),
  (1, 1),
];

/// A priority-flood heap entry. `BinaryHeap` pops the largest entry, so
/// both fields are inverted: the lowest level pops first, and ties go to
/// the lowest index, keeping the flood fully deterministic. The level is
/// stored as its total-order bit pattern, since integer comparisons are
/// much cheaper than `f64::total_cmp`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FloodCell {
  inverted_level: u64,
  inverted_index: u32,
}

/// `value`'s bits rearranged so unsigned comparison gives the same order
/// as `f64::total_cmp`.
pub(crate) fn total_order_bits(value: f64) -> u64 {
  let bits = value.to_bits();

  if bits >> 63 == 1 {
    !bits
  } else {
    bits | (1 << 63)
  }
}

impl FloodCell {
  fn new(level: f64, index: u32) -> Self {
    Self {
      inverted_level: !total_order_bits(level),
      inverted_index: !index,
    }
  }

  fn index(&self) -> u32 {
    !self.inverted_index
  }
}

/// Iterate over the in-bounds eight-way neighbours of `index`.
pub fn neighbours(width: u32, height: u32, index: u32) -> impl Iterator<Item = u32> {
  let x = (index % width) as i32;
  let y = (index / width) as i32;

  OFFSETS.iter().filter_map(move |(dx, dy)| {
    let nx = x + dx;
    let ny = y + dy;

    if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32 {
      None
    } else {
      Some(ny as u32 * width + nx as u32)
    }
  })
}

/// The result of [`priority_flood`].
pub struct Flood {
  /// Heights with every depression filled to its spill level, plus a tiny
  /// gradient so filled flats still drain.
  pub filled: Vec<f64>,
  /// The cell each cell was reached from, which drains it. Outlets have
  /// [`NO_RECEIVER`].
  pub receiver: Vec<u32>,
  /// Cells in the order the flood reached them. Every cell comes after
  /// its receiver, so the reverse is a valid order for accumulation.
  pub order: Vec<u32>,
}

/// Priority-flood depression filling with an epsilon gradient (Barnes,
/// Lehman and Mulla, 2014). Flooding starts from every cell for which
/// `is_outlet` returns true.
pub fn priority_flood(
  width: u32,
  height: u32,
  heights: &[f64],
  epsilon: f64,
  is_outlet: impl Fn(u32) -> bool,
) -> Flood {
  let count = heights.len();
  let mut filled = heights.to_vec();
  let mut receiver = vec![NO_RECEIVER; count];
  let mut visited = vec![false; count];
  let mut order = Vec::with_capacity(count);
  let mut heap = BinaryHeap::new();

  for index in 0..count as u32 {
    if is_outlet(index) {
      visited[index as usize] = true;
      heap.push(FloodCell::new(filled[index as usize], index));
    }
  }

  while let Some(cell) = heap.pop() {
    let index = cell.index();
    let level = filled[index as usize];
    order.push(index);

    for neighbour in neighbours(width, height, index) {
      let n = neighbour as usize;

      if visited[n] {
        continue;
      }

      visited[n] = true;
      filled[n] = heights[n].max(level + epsilon);
      receiver[n] = index;
      heap.push(FloodCell::new(filled[n], neighbour));
    }
  }

  Flood {
    filled,
    receiver,
    order,
  }
}

/// Every cell on the map edge, or at or below `sea`, is an outlet.
pub fn edge_or_sea_outlet(
  width: u32,
  height: u32,
  heights: &[f64],
  sea: f64,
) -> impl Fn(u32) -> bool + '_ {
  move |index| {
    let x = index % width;
    let y = index / width;
    x == 0 || y == 0 || x == width - 1 || y == height - 1 || heights[index as usize] <= sea
  }
}

/// Replace each routed cell's receiver with its steepest downhill
/// neighbour on `surface` (D8), where one exists. This follows valleys
/// more naturally than the flood order alone. Outlets keep
/// [`NO_RECEIVER`].
pub fn steepest_receivers(width: u32, height: u32, surface: &[f64], receiver: &mut [u32]) {
  // Each offset's length, exactly as `sqrt(dx * dx + dy * dy)` gives it.
  let lengths = OFFSETS.map(|(dx, dy)| f64::from(dx * dx + dy * dy).sqrt());

  for y in 0..height {
    // Inside the border every neighbour exists, so they are reached by
    // fixed index steps instead of a division per neighbour.
    let inner_row = y > 0 && y + 1 < height;

    for x in 0..width {
      let index = y * width + x;
      let i = index as usize;

      if receiver[i] == NO_RECEIVER {
        continue;
      }

      let mut best = receiver[i];
      let mut best_drop = 0.0;

      if inner_row && x > 0 && x + 1 < width {
        for ((dx, dy), length) in OFFSETS.iter().zip(lengths) {
          let neighbour = (index as i32 + dy * width as i32 + dx) as u32;
          let drop = (surface[i] - surface[neighbour as usize]) / length;

          if drop > best_drop {
            best_drop = drop;
            best = neighbour;
          }
        }
      } else {
        for neighbour in neighbours(width, height, index) {
          let n = neighbour as usize;
          let dx = (neighbour % width) as f64 - (index % width) as f64;
          let dy = (neighbour / width) as f64 - (index / width) as f64;
          let distance = (dx * dx + dy * dy).sqrt();
          let drop = (surface[i] - surface[n]) / distance;

          if drop > best_drop {
            best_drop = drop;
            best = neighbour;
          }
        }
      }

      receiver[i] = best;
    }
  }
}

/// How much of its transversal deviation a path carries from cell to cell
/// (the `lambda` of Orlandini et al., 2003): 1 is full path memory.
const LTD_MEMORY: f32 = 1.0;

/// Receivers that follow the land instead of the grid, for the river
/// hydrology. `surface` is filled so that every cell drains, and
/// `flood_receiver` and `flood_order` are the receivers and pop order of
/// that flood: its order visits cells in non-decreasing filled height, so
/// its reverse puts every cell before any strictly lower receiver.
///
/// - Flats (cells with no lower neighbour, after pits are filled) drain
///   towards lower ground and away from higher ground at once, by the
///   combined gradient of Barnes, Lehman and Mulla (2014b), after
///   Garbrecht and Martz (1997), with ties broken by a seeded hash, so
///   flow across them converges instead of running in parallel lines.
/// - Every other cell takes the D-infinity direction of steepest descent
///   (Tarboton, 1997), and of the two neighbours that bracket it, the one
///   that keeps the path's accumulated transversal deviation smallest
///   (D8-LTD, Orlandini et al., 2003). A path on a slope between two grid
///   directions alternates between them and follows the slope on average,
///   where steepest-neighbour routing runs straight and then jumps.
///
/// Every receiver is strictly lower, or lower in its flat's order, so the
/// receivers form a tree. Outlets keep [`NO_RECEIVER`].
pub fn land_receivers(
  width: u32,
  height: u32,
  surface: &[f64],
  flood_receiver: &[u32],
  flood_order: &[u32],
  cell_metres: f64,
  seed: u64,
) -> Vec<u32> {
  let count = surface.len();
  let outlet = |i: usize| flood_receiver[i] == NO_RECEIVER;
  let in_bounds = |x: i32, y: i32| x >= 0 && y >= 0 && x < width as i32 && y < height as i32;
  let at = |i: usize| ((i as u32 % width) as i32, (i as u32 / width) as i32);
  let neighbour = |i: usize, (dx, dy): (i32, i32)| {
    let (x, y) = at(i);
    in_bounds(x + dx, y + dy).then(|| ((y + dy) as u32 * width + (x + dx) as u32) as usize)
  };
  let undrained: Vec<bool> = (0..count)
    .map(|i| {
      !outlet(i) && !neighbours(width, height, i as u32).any(|n| surface[n as usize] < surface[i])
    })
    .collect();
  let value = flat_values(&undrained, surface, &neighbour);
  let mut receiver = flood_receiver.to_vec();

  for i in 0..count {
    if !undrained[i] {
      continue;
    }

    // The neighbour at the flat's level with the smallest value; a drained
    // neighbour or outlet there is the flat's way out. Edge and diagonal
    // steps count alike, so paths across a flat do not all run along the
    // rows, and the seed decides between equals.
    let mut best: Option<(i64, u64, usize)> = None;

    for (k, offset) in OFFSETS.iter().enumerate() {
      let Some(n) = neighbour(i, *offset) else {
        continue;
      };

      if surface[n] != surface[i] {
        continue;
      }

      let target = if undrained[n] {
        i64::from(value[n])
      } else {
        -1
      };

      if target >= i64::from(value[i]) {
        continue;
      }

      let tie =
        crate::maths::hash_u64(seed ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ k as u64);

      if best.is_none_or(|(v, t, _)| target < v || (target == v && tie < t)) {
        best = Some((target, tie, n));
      }
    }

    if let Some((_, _, n)) = best {
      receiver[i] = n as u32;
    }
  }

  // Donors first: the reversed flood order, with each run of cells at one
  // level put in flat order (highest value first), then the drained cells
  // the flats spill into.
  let mut order: Vec<u32> = flood_order.iter().rev().copied().collect();
  let mut start = 0;

  while start < order.len() {
    let level = surface[order[start] as usize];
    let mut end = start + 1;

    while end < order.len() && surface[order[end] as usize] == level {
      end += 1;
    }

    if end - start > 1 {
      // Highest key first: flat cells by descending value, then the rest.
      // A heap of plain u64 keys shares its code with the flood's.
      let mut heap: BinaryHeap<u64> = order[start..end]
        .iter()
        .map(|cell| {
          let c = *cell as usize;
          (u64::from(undrained[c]) << 63) | (u64::from(value[c]) << 32) | u64::from(*cell)
        })
        .collect();

      for slot in &mut order[start..end] {
        *slot = heap.pop().map_or(0, |key| key as u32);
      }
    }

    start = end;
  }

  // Each drained cell's two candidates and their offsets, found in map
  // order so the ground is read from memory in rows; the walk below, in
  // flood order, then only reads these.
  let choices: Vec<Choice> = (0..count)
    .map(|i| {
      if outlet(i) || undrained[i] {
        return Choice::NONE;
      }

      // Every neighbour in the map lies in a facet whose corners are all
      // in it, so a drained cell always has one that falls.
      least_deviation(i, (width, height), surface, cell_metres).unwrap_or(Choice::NONE)
    })
    .collect();
  // Upstream area, the largest donor's area so far, and the deviation it
  // brought.
  let mut flow = vec![(1.0f32, 0.0f32, 0.0f32); count];

  for cell in order {
    let i = cell as usize;
    let choice = choices[i];
    let (area, _, carried) = flow[i];
    let carried = LTD_MEMORY * carried;
    let mut passed = 0.0;

    if choice.edge != NO_RECEIVER {
      let (via_edge, via_corner) = (carried + choice.via_edge, carried + choice.via_corner);
      let (r, d) = if choice.edge == choice.corner || via_edge.abs() <= via_corner.abs() {
        (choice.edge, via_edge)
      } else {
        (choice.corner, via_corner)
      };
      receiver[i] = r;
      passed = d;
    }

    let r = receiver[i];

    if r == NO_RECEIVER || outlet(i) {
      continue;
    }

    let slot = &mut flow[r as usize];
    slot.0 += area;

    if area > slot.1 {
      slot.1 = area;
      slot.2 = passed;
    }
  }

  receiver
}

/// Values that order each flat (see [`land_receivers`]): `2 x low + high`,
/// where `low` counts steps from the flat's low edge (cells beside a
/// drained cell or outlet at its level) and `high` is `H - steps from its
/// high edge` (cells beside higher ground), `H` the flat's largest such
/// count; `high` is 0 on a flat with no higher ground around it. Steps are
/// 2 along an edge and 3 along a diagonal. Water moves to smaller values;
/// values of different flats are never compared.
fn flat_values(
  undrained: &[bool],
  surface: &[f64],
  neighbour: &dyn Fn(usize, (i32, i32)) -> Option<usize>,
) -> Vec<u32> {
  let count = undrained.len();
  let mut value = vec![0u32; count];

  if !undrained.contains(&true) {
    return value;
  }

  let around = |i: usize| OFFSETS.iter().filter_map(move |o| neighbour(i, *o));
  // Distance across the flat from its edges, from every edge cell at
  // once: flats at different levels never touch. Steps are counted with a
  // 2-3 chamfer (an edge step 2, a diagonal 3), close to true distance, so
  // water crosses a flat straight towards its way out instead of wandering
  // between the grid directions.
  let spread = |seeds: &dyn Fn(usize) -> bool| {
    let mut steps = vec![0u32; count];
    // Lowest distance first: keys are inverted, as in the flood.
    let key = |at: u32, i: usize| !((u64::from(at) << 32) | i as u64);
    let mut heap = BinaryHeap::new();

    for i in (0..count).filter(|i| undrained[*i] && seeds(*i)) {
      steps[i] = 2;
      heap.push(key(2, i));
    }

    while let Some(entry) = heap.pop() {
      let (at, i) = ((!entry >> 32) as u32, (!entry & 0xffff_ffff) as usize);

      if at > steps[i] {
        continue;
      }

      for (k, offset) in OFFSETS.iter().enumerate() {
        let Some(n) = neighbour(i, *offset) else {
          continue;
        };
        let next = at + if k < 4 { 2 } else { 3 };

        if undrained[n] && surface[n] == surface[i] && (steps[n] == 0 || next < steps[n]) {
          steps[n] = next;
          heap.push(key(next, n));
        }
      }
    }

    steps
  };
  // Outlets are never undrained, so the low edge is beside any drained
  // cell or outlet at the flat's level.
  let low = spread(&|i| around(i).any(|n| surface[n] == surface[i] && !undrained[n]));
  let high = spread(&|i| around(i).any(|n| surface[n] > surface[i]));
  // `H` is the same for every cell of a flat, and values are only ever
  // compared within one flat, so a constant stands in for it.
  const H: u32 = 1 << 30;

  for i in 0..count {
    if undrained[i] {
      value[i] = 2 * low[i] + if high[i] > 0 { H - high[i] } else { 0 };
    }
  }

  value
}

/// The eight triangular facets around a cell, each an edge neighbour and
/// the diagonal neighbour beside it, as indices into [`OFFSETS`].
const FACETS: [(usize, usize); 8] = [
  (1, 7),
  (1, 5),
  (3, 6),
  (3, 7),
  (0, 4),
  (0, 6),
  (2, 5),
  (2, 4),
];

/// A drained cell's two candidate receivers under D8-LTD: the edge and
/// diagonal neighbours bracketing its D-infinity direction, and each one's
/// signed distance in metres from the flow line through the cell. Where
/// the direction lies on a grid direction, both are that neighbour.
#[derive(Clone, Copy)]
struct Choice {
  edge: u32,
  corner: u32,
  via_edge: f32,
  via_corner: f32,
}

impl Choice {
  const NONE: Self = Self {
    edge: NO_RECEIVER,
    corner: NO_RECEIVER,
    via_edge: 0.0,
    via_corner: 0.0,
  };
}

/// The D-infinity direction of drained cell `i` (Tarboton, 1997) as a
/// [`Choice`]; `None` where no facet falls (only at the map edge).
fn least_deviation(
  i: usize,
  (width, height): (u32, u32),
  surface: &[f64],
  cell_metres: f64,
) -> Option<Choice> {
  let g = cell_metres;
  let e0 = surface[i];
  let (x, y) = ((i as u32 % width) as i32, (i as u32 / width) as i32);
  let inside = x > 0 && y > 0 && x + 1 < width as i32 && y + 1 < height as i32;
  // Each neighbour's index and fall from this cell, read once; missing
  // neighbours (off the map) fall by NaN, which no comparison passes.
  let mut index = [0usize; 8];
  let mut fall = [f64::NAN; 8];

  for (k, (dx, dy)) in OFFSETS.iter().enumerate() {
    let (nx, ny) = (x + dx, y + dy);

    if inside || (nx >= 0 && ny >= 0 && nx < width as i32 && ny < height as i32) {
      index[k] = (i as isize + *dy as isize * width as isize + *dx as isize) as usize;
      fall[k] = e0 - surface[index[k]];
    }
  }

  // The steepest facet: its squared fall (to compare without a square
  // root), its falls along and across its edge, and which facet it is.
  // Falls are per cell, which orders the facets as falls per metre do.
  let (mut steepest, mut a, mut b, mut which) = (0.0f64, 0.0f64, 0.0f64, usize::MAX);

  for (k, (e, c)) in FACETS.iter().enumerate() {
    let (d1, d2) = (fall[*e], fall[*c] - fall[*e]);
    // Within the facet (under 45 degrees from its edge) where both fall,
    // else along its edge or its diagonal.
    let within = d1 > 0.0 && d2 > 0.0 && d2 < d1;
    let along_edge = d1 > 0.0 && d2 <= 0.0;
    let diagonal = fall[*c].max(0.0);
    let steepness = if within {
      d1 * d1 + d2 * d2
    } else if along_edge {
      d1 * d1
    } else {
      0.5 * diagonal * diagonal
    };

    if steepness > steepest {
      steepest = steepness;
      which = k;
      (a, b) = if within {
        (d1, d2)
      } else if along_edge {
        (1.0, 0.0)
      } else {
        (1.0, 1.0)
      };
    }
  }

  if which == usize::MAX {
    return None;
  }

  let (e, c) = FACETS[which];
  let (edge, corner) = (OFFSETS[e], OFFSETS[c]);
  let (n1, n2) = (index[e], index[c]);
  let only = |n: usize| Choice {
    edge: n as u32,
    corner: n as u32,
    via_edge: 0.0,
    via_corner: 0.0,
  };

  if b <= 0.0 {
    return Some(only(n1));
  }

  if b >= a {
    return Some(only(n2));
  }

  // The true flow direction is the facet's gradient, `a` along the edge
  // and `b` across it, so each candidate's signed distance from the flow
  // line needs no angle.
  let side = (f64::from(corner.0 - edge.0), f64::from(corner.1 - edge.1));
  let length = (a * a + b * b).sqrt();
  let flow = (
    (f64::from(edge.0) * a + side.0 * b) / length,
    (f64::from(edge.1) * a + side.1 * b) / length,
  );
  let offset =
    |(dx, dy): (i32, i32)| (g * (flow.0 * f64::from(dy) - flow.1 * f64::from(dx))) as f32;
  Some(Choice {
    edge: n1 as u32,
    corner: n2 as u32,
    via_edge: offset(edge),
    via_corner: offset(corner),
  })
}

/// Follow the receiver chain from `start`, calling `step` with each
/// receiver in turn until it returns `false` or the chain reaches an
/// outlet. Receivers form trees by construction, so a chain is never
/// longer than the grid; a walk that gets that far has found a cycle, and
/// stops with an error instead of running on.
pub fn walk_receivers(
  receiver: &[u32],
  start: u32,
  mut step: impl FnMut(u32) -> bool,
) -> Result<(), String> {
  let mut cell = start;

  for _ in 0..receiver.len() {
    let next = receiver.get(cell as usize).copied().unwrap_or(NO_RECEIVER);

    if next == NO_RECEIVER || !step(next) {
      return Ok(());
    }

    cell = next;
  }

  Err(format!(
    "the receivers from cell {start} form a cycle: the walk passed {} steps",
    receiver.len()
  ))
}

/// Whether every receiver chain reaches an outlet: the graph is a forest,
/// as routing must leave it. The stack order holds every cell exactly
/// when no chain is caught in a cycle.
pub fn receivers_acyclic(receiver: &[u32]) -> bool {
  stack_order(receiver).len() == receiver.len()
}

/// Order cells so every cell comes after its receiver (the "stack" of
/// Braun and Willett, 2013), walking up the receiver trees from the
/// outlets. Cells whose receiver chain never reaches an outlet are left
/// out, which cannot happen for receivers from [`priority_flood`].
pub fn stack_order(receiver: &[u32]) -> Vec<u32> {
  let mut order = Vec::with_capacity(receiver.len());
  StackOrder::default().order(receiver, &mut order);
  order
}

/// Reusable buffers for [`stack_order`], for callers that order the same
/// grid many times.
#[derive(Default)]
pub struct StackOrder {
  /// Donors of every cell, grouped by receiver in cell order.
  donors: Vec<u32>,
  /// Where each cell's donors start in `donors`, plus one end.
  start: Vec<u32>,
  /// Where the next donor of each cell goes while `donors` fills.
  next: Vec<u32>,
}

impl StackOrder {
  /// Write the stack order of `receiver` into `order`.
  pub fn order(&mut self, receiver: &[u32], order: &mut Vec<u32>) {
    let count = receiver.len();
    self.start.clear();
    self.start.resize(count + 1, 0);
    order.clear();

    for r in receiver.iter().filter(|r| **r != NO_RECEIVER) {
      self.start[*r as usize + 1] += 1;
    }

    for index in 0..count {
      self.start[index + 1] += self.start[index];
    }

    self.next.clear();
    self.next.extend_from_slice(&self.start[..count]);
    self.donors.resize(count, 0);

    for (index, r) in receiver.iter().enumerate() {
      if *r == NO_RECEIVER {
        order.push(index as u32);
      } else {
        let slot = &mut self.next[*r as usize];
        self.donors[*slot as usize] = index as u32;
        *slot += 1;
      }
    }

    // Breadth-first from the outlets, using `order` itself as the queue.
    let mut head = 0;

    while head < order.len() {
      let c = order[head] as usize;
      head += 1;
      order.extend_from_slice(&self.donors[self.start[c] as usize..self.start[c + 1] as usize]);
    }
  }
}

/// Upstream drainage area on a grid of flow cells, in heightmap samples:
/// how much of the map drains through each cell, itself included.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrainageArea {
  /// Grid width in cells.
  pub width: u32,
  /// Grid height in cells.
  pub height: u32,
  /// Heightmap samples per cell along each axis.
  pub stride: u32,
  /// Upstream area of each cell, in cells.
  pub cells: Vec<f32>,
}

impl DrainageArea {
  /// Upstream area, in samples, at heightmap sample `(x, y)`; 1 where
  /// there is no grid.
  pub fn at(&self, x: u32, y: u32) -> f32 {
    if self.cells.is_empty() {
      return 1.0;
    }

    let cx = ((x + self.stride / 2) / self.stride).min(self.width - 1);
    let cy = ((y + self.stride / 2) / self.stride).min(self.height - 1);
    self.cells[(cy * self.width + cx) as usize] * (self.stride * self.stride) as f32
  }
}

impl DrainageArea {
  /// D8 drainage on the final heights, for tree placement when there is
  /// no river hydrology: each cell drains to its steepest downhill
  /// neighbour, and pits and flats keep what reaches them. There is no
  /// depression filling, so it costs a few milliseconds even on a
  /// 512 x 512 map. Maps over 1,024 samples a side are read on a coarser
  /// grid.
  pub fn d8(width: u32, height: u32, heights: &[f32]) -> Self {
    if width == 0 || height == 0 || heights.len() != (width * height) as usize {
      return Self::default();
    }

    let stride = (width.max(height) - 1) / 1024 + 1;
    let (w, h) = ((width - 1) / stride + 1, (height - 1) / stride + 1);
    let surface: Vec<f64> = (0..w * h)
      .map(|cell| f64::from(heights[((cell / w) * stride * width + (cell % w) * stride) as usize]))
      .collect();
    // Every cell starts as its own receiver; those with no downhill
    // neighbour become outlets.
    let mut receiver: Vec<u32> = (0..w * h).collect();
    steepest_receivers(w, h, &surface, &mut receiver);

    for (index, r) in receiver.iter_mut().enumerate() {
      if *r == index as u32 {
        *r = NO_RECEIVER;
      }
    }

    let order = stack_order(&receiver);
    Self {
      width: w,
      height: h,
      stride,
      cells: accumulate(&order, &receiver, vec![1.0; (w * h) as usize]),
    }
  }
}

/// Accumulate `weights` downstream: each cell's result is its own weight
/// plus the results of every cell that drains into it. `order` must list
/// every cell after its receiver.
pub fn accumulate(order: &[u32], receiver: &[u32], weights: Vec<f32>) -> Vec<f32> {
  let mut accumulation = weights;
  accumulate_into(order, receiver, &mut accumulation);
  accumulation
}

/// [`accumulate`] in place: `accumulation` holds the weights on entry.
pub fn accumulate_into(order: &[u32], receiver: &[u32], accumulation: &mut [f32]) {
  for index in order.iter().rev() {
    let i = *index as usize;
    let r = receiver[i];

    if r != NO_RECEIVER {
      accumulation[r as usize] += accumulation[i];
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn d8_drainage_gathers_down_a_valley() {
    // A V-shaped valley down the middle column, falling to the south.
    let (width, height) = (33u32, 40u32);
    let heights: Vec<f32> = (0..width * height)
      .map(|i| {
        let (x, y) = ((i % width) as f32, (i / width) as f32);
        (x - 16.0).abs() * 2.0 + (height as f32 - y) * 0.5
      })
      .collect();
    let area = DrainageArea::d8(width, height, &heights);

    assert_eq!((area.width, area.height, area.stride), (width, height, 1));
    // The valley floor gathers the whole valley above it; ridges only
    // themselves.
    assert!(area.at(16, 39) > 1000.0, "{}", area.at(16, 39));
    assert_eq!(area.at(0, 20), 1.0);
    assert!(area.at(16, 20) > area.at(16, 10));
  }

  #[test]
  fn flood_fills_a_pit_to_its_spill_level() {
    // A 5 x 5 bowl whose rim is 10 m with one 4 m notch on the left edge.
    let mut heights = vec![10.0; 25];
    heights[12] = 1.0;
    heights[11] = 5.0;
    heights[10] = 4.0;
    let flood = priority_flood(
      5,
      5,
      &heights,
      1e-3,
      edge_or_sea_outlet(5, 5, &heights, -1.0),
    );

    assert!(flood.filled[12] > 5.0 && flood.filled[12] < 5.01);
    assert_eq!(flood.receiver[12], 11);
    assert_eq!(flood.receiver[11], 10);
    assert_eq!(flood.order.len(), 25);
  }

  #[test]
  fn stack_order_puts_receivers_first_and_accumulation_counts_cells() {
    // A tilted 4 x 4 plane draining to the left edge.
    let heights: Vec<f64> = (0..16).map(|i| (i % 4) as f64).collect();
    let flood = priority_flood(4, 4, &heights, 1e-6, |i| i % 4 == 0);
    let mut receiver = flood.receiver.clone();
    steepest_receivers(4, 4, &flood.filled, &mut receiver);
    let order = stack_order(&receiver);
    let mut position = [0; 16];

    for (at, cell) in order.iter().enumerate() {
      position[*cell as usize] = at;
    }

    for (cell, r) in receiver.iter().enumerate() {
      if *r != NO_RECEIVER {
        assert!(position[*r as usize] < position[cell]);
      }
    }

    let area = accumulate(&order, &receiver, vec![1.0; 16]);
    let outlets: f32 = (0..16).filter(|i| i % 4 == 0).map(|i| area[i]).sum();
    assert_eq!(outlets, 16.0);
  }

  /// [`land_receivers`] on `heights`, flooded from the cells `is_outlet`
  /// names, with no gradient across filled pits.
  fn routed(size: u32, heights: &[f64], is_outlet: impl Fn(u32) -> bool, seed: u64) -> Vec<u32> {
    let flood = priority_flood(size, size, heights, 0.0, is_outlet);
    land_receivers(
      size,
      size,
      &flood.filled,
      &flood.receiver,
      &flood.order,
      10.0,
      seed,
    )
  }

  #[test]
  fn a_receiver_cycle_stops_the_walk_at_its_bound_with_an_error() {
    // 0 -> 1 -> 2 -> 0, and 3 drains out.
    let receiver = [1, 2, 0, NO_RECEIVER];
    let mut steps = 0;
    let walked = walk_receivers(&receiver, 0, |_| {
      steps += 1;
      true
    });
    assert!(walked.unwrap_err().contains("form a cycle"));
    assert_eq!(steps, receiver.len());
    assert!(!receivers_acyclic(&receiver));

    let forest = [1, NO_RECEIVER, 1, 2];
    let mut path = Vec::new();
    walk_receivers(&forest, 3, |cell| {
      path.push(cell);
      true
    })
    .unwrap();
    assert_eq!(path, [2, 1]);
    assert!(receivers_acyclic(&forest));
  }

  /// Follow receivers from `start` for at most `steps` cells.
  fn path(receiver: &[u32], start: u32, steps: usize) -> Vec<u32> {
    let mut cells = vec![start];

    while cells.len() <= steps {
      let r = receiver[*cells.last().unwrap() as usize];

      if r == NO_RECEIVER {
        break;
      }

      cells.push(r);
    }

    cells
  }

  #[test]
  fn least_deviation_paths_follow_a_slope_between_grid_directions() {
    let size = 200u32;

    for degrees in [22.5f64, 30.0, 60.0] {
      let (sin, cos) = degrees.to_radians().sin_cos();
      let heights: Vec<f64> = (0..size * size)
        .map(|i| {
          let (x, y) = (f64::from(i % size), f64::from(i / size));
          500.0 - (x * cos + y * sin) * 0.8
        })
        .collect();
      let on_edge = |i: u32| {
        let (x, y) = (i % size, i / size);
        x == 0 || y == 0 || x == size - 1 || y == size - 1
      };
      let start = 20 * size + 20;
      let cells = path(&routed(size, &heights, on_edge, 1), start, 60);
      assert_eq!(cells.len(), 61, "{degrees} degrees: the path stopped");
      let xy = |c: u32| (f64::from(c % size), f64::from(c / size));
      let (x0, y0) = xy(start);
      let (x1, y1) = xy(*cells.last().unwrap());
      let heading = (y1 - y0).atan2(x1 - x0).to_degrees();
      assert!(
        (heading - degrees).abs() <= 3.0,
        "{degrees} degrees: heading {heading}"
      );

      for cell in &cells {
        let (x, y) = xy(*cell);
        let off = ((x - x0) * sin - (y - y0) * cos).abs();
        assert!(off <= 1.5, "{degrees} degrees: {off} cells off the line");
      }
    }

    // D8 runs along the nearer grid direction instead.
    let (sin, cos) = 30f64.to_radians().sin_cos();
    let heights: Vec<f64> = (0..size * size)
      .map(|i| 500.0 - (f64::from(i % size) * cos + f64::from(i / size) * sin) * 0.8)
      .collect();
    let flood = priority_flood(size, size, &heights, 0.0, |i| i % size == size - 1);
    let mut d8 = flood.receiver.clone();
    steepest_receivers(size, size, &flood.filled, &mut d8);
    let end = *path(&d8, 20 * size + 20, 60).last().unwrap();
    let heading = f64::from(end / size - 20)
      .atan2(f64::from(end % size - 20))
      .to_degrees();
    assert!((heading - 30.0).abs() > 3.0, "D8 heading {heading}");
  }

  /// A flat plateau 64 cells across at 10 m, ringed by 20 m ground, with
  /// one 5 m outlet in the middle of its west side.
  fn plateau() -> (u32, Vec<f64>, u32) {
    let size = 66u32;
    let outlet = 33 * size;
    let heights = (0..size * size)
      .map(|i| {
        let (x, y) = (i % size, i / size);

        if i == outlet {
          5.0
        } else if x == 0 || y == 0 || x == size - 1 || y == size - 1 {
          20.0
        } else {
          10.0
        }
      })
      .collect();
    (size, heights, outlet)
  }

  #[test]
  fn flats_drain_to_their_outlet_by_converging_paths() {
    let (size, heights, outlet) = plateau();
    let receiver = routed(size, &heights, |i| i == outlet, 7);
    let ring = |c: u32| {
      let (x, y) = (c % size, c / size);
      x == 1 || y == 1 || x == size - 2 || y == size - 2
    };
    let (mut length, mut straight) = (0.0, 0.0);

    for y in 1..size - 1 {
      for x in 1..size - 1 {
        let start = y * size + x;
        let cells = path(&receiver, start, 10_000);
        assert_eq!(
          *cells.last().unwrap(),
          outlet,
          "{x}, {y} does not reach the outlet"
        );
        let mut along_edge = 0;

        for pair in cells.windows(2) {
          let (a, b) = (pair[0], pair[1]);
          let (dx, dy) = (
            f64::from(b % size) - f64::from(a % size),
            f64::from(b / size) - f64::from(a / size),
          );
          length += dx.hypot(dy);
          along_edge = if ring(a) && ring(b) {
            along_edge + 1
          } else {
            0
          };
          assert!(along_edge <= 3, "{x}, {y} runs along the high edge");
        }

        straight += (f64::from(x)).hypot(f64::from(y) - 33.0);
      }
    }

    assert!(length <= straight * 1.15, "{length} against {straight}");
  }

  #[test]
  fn flat_tie_breaks_are_seeded_and_only_on_flats() {
    let (size, mut heights, outlet) = plateau();
    // A slope in the south half, so some cells are not on the flat.
    for y in 40..size - 1 {
      for x in 1..size - 1 {
        heights[(y * size + x) as usize] = 10.0 + f64::from(y - 39) * 0.5 + f64::from(x) * 0.01;
      }
    }

    let first = routed(size, &heights, |i| i == outlet, 7);
    assert_eq!(first, routed(size, &heights, |i| i == outlet, 7));
    let other = routed(size, &heights, |i| i == outlet, 8);
    assert_ne!(first, other);

    for (cell, (a, b)) in first.iter().zip(&other).enumerate() {
      if a != b {
        assert!(heights[cell] == 10.0, "cell {cell} changed off the flat");
      }
    }
  }
}
