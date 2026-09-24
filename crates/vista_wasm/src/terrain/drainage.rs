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
  for index in 0..surface.len() as u32 {
    let i = index as usize;

    if receiver[i] == NO_RECEIVER {
      continue;
    }

    let mut best = receiver[i];
    let mut best_drop = 0.0;

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

    receiver[i] = best;
  }
}

/// Route every non-outlet cell to its steepest downhill neighbour (D8),
/// or return `None` if some cell has no lower neighbour. On a surface
/// with no depressions this gives the same receivers as
/// [`priority_flood`] followed by [`steepest_receivers`], without the
/// flood.
pub fn steepest_if_drained(
  width: u32,
  height: u32,
  heights: &[f64],
  is_outlet: impl Fn(u32) -> bool,
) -> Option<Vec<u32>> {
  let mut receiver = vec![NO_RECEIVER; heights.len()];

  for index in 0..heights.len() as u32 {
    if is_outlet(index) {
      continue;
    }

    let i = index as usize;
    let mut best = NO_RECEIVER;
    let mut best_drop = 0.0;

    for neighbour in neighbours(width, height, index) {
      let n = neighbour as usize;
      let dx = (neighbour % width) as f64 - (index % width) as f64;
      let dy = (neighbour / width) as f64 - (index / width) as f64;
      let drop = (heights[i] - heights[n]) / (dx * dx + dy * dy).sqrt();

      if drop > best_drop {
        best_drop = drop;
        best = neighbour;
      }
    }

    if best == NO_RECEIVER {
      return None;
    }

    receiver[i] = best;
  }

  Some(receiver)
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
  /// Donors of each cell, in eight fixed slots per cell.
  donors: Vec<u32>,
  /// How many slots of each cell are used.
  counts: Vec<u8>,
}

impl StackOrder {
  /// Write the stack order of `receiver` into `order`. Every receiver
  /// must be one of its cell's eight neighbours, so no cell has more than
  /// eight donors.
  pub fn order(&mut self, receiver: &[u32], order: &mut Vec<u32>) {
    let count = receiver.len();
    self.counts.clear();
    self.counts.resize(count, 0);
    self.donors.resize(count * 8, 0);
    order.clear();

    for (index, r) in receiver.iter().enumerate() {
      if *r == NO_RECEIVER {
        order.push(index as u32);
      } else {
        let r = *r as usize;
        let slot = self.counts[r] as usize;
        self.donors[r * 8 + slot] = index as u32;
        self.counts[r] += 1;
      }
    }

    // Breadth-first from the outlets, using `order` itself as the queue.
    let mut head = 0;

    while head < order.len() {
      let c = order[head] as usize;
      head += 1;

      for slot in 0..self.counts[c] as usize {
        order.push(self.donors[c * 8 + slot]);
      }
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
}
