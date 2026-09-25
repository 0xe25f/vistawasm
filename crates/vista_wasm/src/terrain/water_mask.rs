//! Painted water: a host's mask of rivers and lakes, carved and drawn like
//! the terrain's own.
//!
//! Lakes are flattened into basins below their rim, so the drainage fills
//! them to their lowest rim point and they overflow there into the
//! natural network. Rivers are thinned to centrelines, turned to run
//! downhill, and handed to the channel stage, which cuts them at their
//! painted width (or wider, where their discharge asks for it) and joins
//! them to the natural network. Every change is recorded, so clearing the
//! mask restores the terrain exactly.

use vista_types::WaterMask;

use crate::errors::{VistaError, VistaResult};
use crate::maths::smoothstep;
use crate::terrain::channels::CarveRecord;
use crate::terrain::drainage::neighbours;
use crate::terrain::HeightMap;

/// Mask values from this up are lakes; below it, from 1, rivers.
pub const LAKE: u8 = 128;

/// A painted river centreline, from its upstream end.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintedRiver {
  /// Heightmap sample coordinates.
  pub points: Vec<[f32; 2]>,
  /// Painted width in metres.
  pub width: f32,
}

/// Check a mask's size and data.
pub fn validate(mask: &WaterMask) -> VistaResult<()> {
  if !(2..=8192).contains(&mask.width) || !(2..=8192).contains(&mask.height) {
    return Err(VistaError::options(format!(
      "setWaterMask width and height must be from 2 to 8192, but they are {} and {}.",
      mask.width, mask.height
    )));
  }

  let expected = mask.width as usize * mask.height as usize;

  if mask.data.len() != expected {
    return Err(VistaError::options(format!(
      "setWaterMask data must hold width x height = {expected} bytes, but it holds {}.",
      mask.data.len()
    )));
  }

  Ok(())
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

    // The bed deepens away from the shore over three samples, to the
    // basin depth below the rim.
    let depth = (0.05 * (cells.len() as f32 * metres * metres).sqrt()).max(1.5);
    let mut from_shore = vec![u32::MAX; cells.len()];
    let mut queue: Vec<usize> = Vec::new();

    for (slot, cell) in cells.iter().enumerate() {
      slot_of[*cell as usize] = slot as u32;
    }

    for (slot, cell) in cells.iter().enumerate() {
      if neighbours(width, height, *cell).any(|n| region[n as usize] != start) {
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

    for (slot, cell) in cells.iter().enumerate() {
      let i = *cell as usize;

      if !map.no_data[i] {
        let deep = 0.3 + 0.7 * smoothstep(from_shore[slot] as f32 / 3.0);
        record.lower(map, i, rim - depth * deep);
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
  // Neighbours clockwise from north: P2 to P9.
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

          let p = RING.map(|(dx, dy)| at(&on, x + dx, y + dy));
          let b = p.iter().filter(|v| **v).count();
          let a = (0..8).filter(|i| !p[*i] && p[(i + 1) % 8]).count();
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

  // Trace from every end, then around any closed loop left over.
  let degree = |on: &[bool], x: i32, y: i32| {
    RING
      .iter()
      .filter(|(dx, dy)| at(on, x + dx, y + dy))
      .count()
  };
  let mut visited = vec![false; on.len()];
  let mut rivers = Vec::new();
  let starts: Vec<(i32, i32)> = (0..height)
    .flat_map(|y| (0..width).map(move |x| (x, y)))
    .filter(|&(x, y)| at(&on, x, y))
    .collect();
  let ends = starts.iter().filter(|&&(x, y)| degree(&on, x, y) == 1);
  let others = starts.iter().filter(|&&(x, y)| degree(&on, x, y) != 1);

  for &(x0, y0) in ends.chain(others) {
    if visited[(y0 * width + x0) as usize] {
      continue;
    }

    let mut line = vec![(x0, y0)];
    visited[(y0 * width + x0) as usize] = true;
    let (mut x, mut y) = (x0, y0);

    while degree(&on, x, y) <= 2 || line.len() == 1 {
      let Some(&(dx, dy)) = RING
        .iter()
        .find(|(dx, dy)| at(&on, x + dx, y + dy) && !visited[((y + dy) * width + x + dx) as usize])
      else {
        break;
      };
      x += dx;
      y += dy;
      visited[(y * width + x) as usize] = true;
      line.push((x, y));
    }

    if line.len() < 2 {
      continue;
    }

    let strength = line
      .iter()
      .map(|&(x, y)| painted[(y * width + x) as usize] as f32)
      .fold(1.0, f32::max);
    let heights: Vec<f32> = line
      .iter()
      .map(|&(x, y)| map.heights[(y * width + x) as usize])
      .collect();
    let (first, last) = (heights[0], heights[heights.len() - 1]);
    let downhill = if (first - last).abs() > 1e-3 {
      first > last
    } else {
      // Level ends: flow towards the nearer sea or lake.
      let (a, b) = (line[0], line[line.len() - 1]);
      towards_water(map, painted, a) > towards_water(map, painted, b)
    };

    if !downhill {
      line.reverse();
    }

    rivers.push(PaintedRiver {
      points: line.iter().map(|&(x, y)| [x as f32, y as f32]).collect(),
      width: 1.0 + (strength - 1.0) / 126.0 * 59.0,
    });
  }

  rivers
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
