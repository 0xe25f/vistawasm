//! Where water can be heard, for hosts that play their own audio.
//!
//! River segments (strength: speed times width), waterfalls (discharge
//! times drop), lake shores and the coast (wave height) are sorted into
//! one grid per kind, built once with the rivers. Each grid's cells are as
//! large as its search radius, so a query reads only the nine cells
//! around the listener and never allocates. Frozen water is silent.

use vista_types::{WaterSound, WaterSounds};

use crate::maths::length2;
use crate::render::water::{
  freeze_fraction, RiverNetwork, FALL_FREEZE_CELSIUS, LAKE_FREEZE_CELSIUS, RIVER_FREEZE_CELSIUS,
};
use crate::terrain::channels::ReachKind;
use crate::terrain::HeightMap;

/// Search radius in metres: rivers, waterfalls, lake shores and surf.
const RADIUS: [f32; 4] = [400.0, 1500.0, 400.0, 600.0];

/// Squared distance, in square metres, at which a source of strength 1
/// is half as loud as at its side.
const HALF_LOUD: [f32; 4] = [720.0, 4_500.0, 900.0, 22_500.0];

/// Sources are kept at least this far apart along a river or a shore.
const SPACING_METRES: f32 = 16.0;

#[derive(Clone, Copy, Debug, Default)]
struct Source {
  position: [f32; 3],
  strength: f32,
}

/// Items sorted into square cells laid over the terrain, so a query reads
/// only the cells around a point and never allocates. Water sounds use
/// one per kind of source; tree placement uses one of river segments.
#[derive(Clone, Debug, Default)]
pub struct BucketGrid<T> {
  origin: [f32; 2],
  cell: f32,
  columns: i32,
  rows: i32,
  /// Where each cell's items start in `items`, plus one end.
  start: Vec<u32>,
  items: Vec<T>,
}

/// Most cells a [`BucketGrid`] has: 4 Mi, 16 MiB of cell starts.
const MAX_BUCKET_CELLS: f64 = (1u32 << 22) as f64;

impl<T: Clone + Default> BucketGrid<T> {
  /// Sort `items` into cells `cell` metres wide covering the terrain of
  /// half extents `half`, each at the world (x, z) `position` gives.
  /// Items beyond the terrain go in its border cells.
  pub fn build(
    half: [f32; 2],
    cell: f32,
    items: Vec<T>,
    position: impl Fn(&T) -> [f32; 2],
  ) -> Self {
    let positions: Vec<[f32; 2]> = items.iter().map(position).collect();
    let (grid, order) = BucketGrid::<u32>::cells(half, cell, &positions);

    BucketGrid {
      origin: grid.origin,
      cell: grid.cell,
      columns: grid.columns,
      rows: grid.rows,
      start: grid.start,
      items: order.iter().map(|&i| items[i as usize].clone()).collect(),
    }
  }
}

impl BucketGrid<u32> {
  /// The grid's cells for items at `positions`, without the items, and the
  /// item order that groups them by cell. It does not depend on the item
  /// type, so there is one copy of it however many kinds are sorted.
  fn cells(half: [f32; 2], cell: f32, positions: &[[f32; 2]]) -> (Self, Vec<u32>) {
    // A terrain kilometres a sample across would need billions of cells
    // `cell` wide; there the cells widen until there are at most
    // `MAX_BUCKET_CELLS`. Queries still see every item within a cell.
    let wanted = (f64::from(half[0]) * 2.0 / f64::from(cell) + 2.0)
      * (f64::from(half[1]) * 2.0 / f64::from(cell) + 2.0);
    let cell = if wanted > MAX_BUCKET_CELLS {
      cell * (wanted / MAX_BUCKET_CELLS).sqrt() as f32 * 1.01
    } else {
      cell
    };
    let origin = [-half[0], -half[1]];
    let columns = (half[0] * 2.0 / cell).ceil() as i32 + 1;
    let rows = (half[1] * 2.0 / cell).ceil() as i32 + 1;
    let slot = |[x, z]: [f32; 2]| {
      let x = (((x - origin[0]) / cell) as i32).clamp(0, columns - 1);
      let y = (((z - origin[1]) / cell) as i32).clamp(0, rows - 1);
      (y * columns + x) as usize
    };
    let mut start = vec![0u32; (columns * rows) as usize + 1];

    for position in positions {
      start[slot(*position) + 1] += 1;
    }

    for i in 1..start.len() {
      start[i] += start[i - 1];
    }

    let mut next = start.clone();
    let mut order = vec![0u32; positions.len()];

    for (index, position) in positions.iter().enumerate() {
      let at = &mut next[slot(*position)];
      order[*at as usize] = index as u32;
      *at += 1;
    }

    let grid = BucketGrid {
      origin,
      cell,
      columns,
      rows,
      start,
      items: Vec::new(),
    };
    (grid, order)
  }
}

impl<T> BucketGrid<T> {
  /// The grid's origin (x, z), cell size, columns, rows, where each
  /// cell's items start (plus one end), and the items: everything a GPU
  /// copy of it needs.
  pub fn parts(&self) -> ([f32; 2], f32, i32, i32, &[u32], &[T]) {
    (
      self.origin,
      self.cell,
      self.columns,
      self.rows,
      &self.start,
      &self.items,
    )
  }

  /// Visit every item in the cell holding world `(x, z)` and the eight
  /// around it: all items within one cell width of the point, and some
  /// further.
  pub fn for_each_near(&self, x: f32, z: f32, mut visit: impl FnMut(&T)) {
    let cx = ((x - self.origin[0]) / self.cell).floor() as i32;
    let cy = ((z - self.origin[1]) / self.cell).floor() as i32;
    // Far-off positions saturate the cast; saturating neighbours keep the
    // range empty instead of overflowing.
    let x0 = cx.saturating_sub(1).max(0);
    let x1 = cx.saturating_add(1).min(self.columns - 1);

    if x0 > x1 || self.start.is_empty() {
      return;
    }

    for y in cy.saturating_sub(1).max(0)..=cy.saturating_add(1).min(self.rows - 1) {
      let row = (y * self.columns) as usize;
      let (first, last) = (row + x0 as usize, row + x1 as usize + 1);

      for item in &self.items[self.start[first] as usize..self.start[last] as usize] {
        visit(item);
      }
    }
  }
}

fn grid(kind: usize, half: [f32; 2], sources: Vec<Source>) -> BucketGrid<Source> {
  BucketGrid::build(half, RADIUS[kind], sources, |source| {
    [source.position[0], source.position[2]]
  })
}

/// Water sound sources on a terrain.
#[derive(Clone, Debug, Default)]
pub struct SoundMap {
  grids: Vec<BucketGrid<Source>>,
}

impl SoundMap {
  /// Collect the sources of `map` and its rivers.
  pub fn build(map: &HeightMap, rivers: &RiverNetwork) -> Self {
    let width = map.metadata.width;
    let height = map.metadata.height;

    if width < 2 || height < 2 {
      return Self::default();
    }

    let metres = map.metadata.metres_per_sample.max(0.001);
    let half = [
      (width as f32 - 1.0) * metres * 0.5,
      (height as f32 - 1.0) * metres * 0.5,
    ];
    let world = |x: f32, y: f32, level: f32| [x * metres - half[0], level, y * metres - half[1]];
    let mut kinds: [Vec<Source>; 4] = Default::default();

    // Threads are heard through their belt's own reach.
    for reach in rivers
      .reaches
      .iter()
      .filter(|reach| reach.kind != ReachKind::Thread)
    {
      let mut last = [f32::MAX; 2];

      for point in &reach.points {
        let far = length2(point.x - last[0], point.y - last[1]) * metres >= SPACING_METRES;

        if far && freeze_fraction(point.celsius, RIVER_FREEZE_CELSIUS) < 1.0 && !point.falling {
          last = [point.x, point.y];
          kinds[0].push(Source {
            position: world(point.x, point.y, point.level),
            strength: point.speed * point.width,
          });
        }
      }
    }

    for fall in rivers
      .falls
      .iter()
      .filter(|fall| fall.celsius >= FALL_FREEZE_CELSIUS)
    {
      kinds[1].push(Source {
        position: world(
          fall.foot[0],
          fall.foot[1],
          fall.foot_level + fall.height() * 0.3,
        ),
        strength: fall.discharge * fall.height(),
      });
    }

    for lake in &rivers.lakes {
      if freeze_fraction(lake.celsius, LAKE_FREEZE_CELSIUS) < 1.0 {
        for point in &lake.shore {
          kinds[2].push(Source {
            position: world(point[0], point[1], lake.surface),
            strength: 1.0,
          });
        }
      }
    }

    // One coastal sample per 32 m square: land beside the sea.
    let sea = map.metadata.sea_level_metres;
    let step = ((SPACING_METRES * 2.0 / metres) as u32).max(1);
    let land = |x: u32, y: u32| {
      let index = (y * width + x) as usize;
      !map.no_data[index] && map.heights[index] > sea
    };

    for by in (0..height).step_by(step as usize) {
      for bx in (0..width).step_by(step as usize) {
        let coast = (by..(by + step).min(height))
          .flat_map(|y| (bx..(bx + step).min(width)).map(move |x| (x, y)))
          .find(|&(x, y)| {
            land(x, y)
              && ((x > 0 && !land(x - 1, y))
                || (y > 0 && !land(x, y - 1))
                || (x + 1 < width && !land(x + 1, y))
                || (y + 1 < height && !land(x, y + 1)))
          });

        if let Some((x, y)) = coast {
          kinds[3].push(Source {
            position: world(x as f32, y as f32, sea),
            strength: 1.0,
          });
        }
      }
    }

    Self {
      grids: kinds
        .into_iter()
        .enumerate()
        .map(|(kind, sources)| grid(kind, half, sources))
        .collect(),
    }
  }

  /// The loudest source of each kind near `listener`. Surf is scaled by
  /// `wave_height` in metres.
  pub fn query(&self, listener: [f32; 3], wave_height: f32) -> WaterSounds {
    let mut found: [Option<WaterSound>; 4] = Default::default();

    for (kind, grid) in self.grids.iter().enumerate() {
      let cell = RADIUS[kind];
      let scale = if kind == 3 { wave_height.max(0.0) } else { 1.0 };

      let best = &mut found[kind];

      grid.for_each_near(listener[0], listener[2], |source| {
        let d = [
          source.position[0] - listener[0],
          source.position[1] - listener[1],
          source.position[2] - listener[2],
        ];
        let squared = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];

        if squared > cell * cell {
          return;
        }

        let strength = source.strength * scale * HALF_LOUD[kind];
        let loudness = strength / (strength + squared).max(1e-6);

        if best.as_ref().is_none_or(|best| loudness > best.loudness) {
          *best = Some(WaterSound {
            distance_metres: squared.sqrt(),
            loudness,
            position: source.position,
          });
        }
      });
    }

    let [river, waterfall, lake_shore, surf] = found;
    WaterSounds {
      river,
      waterfall,
      lake_shore,
      surf,
    }
  }
}

#[cfg(test)]
mod tests {
  /// A terrain 80,000 km across (a few samples kilometres apart) would need
  /// billions of 300 m cells; the grid widens its cells instead, and still
  /// finds what is near.
  #[test]
  fn a_vast_terrain_gets_wider_cells_not_billions_of_them() {
    let half = [41_000_000.0, 41_000_000.0];
    let items = vec![[1_000.0f32, 2_000.0], [-30_000_000.0, 5.0]];
    let grid = BucketGrid::build(half, 300.0, items, |item| *item);
    let (_, cell, columns, rows, start, _) = grid.parts();
    assert!(cell > 300.0);
    assert!(columns as f64 * rows as f64 <= MAX_BUCKET_CELLS);
    assert_eq!(start.len(), (columns * rows) as usize + 1);
    let mut near = Vec::new();
    grid.for_each_near(1_000.0, 2_000.0, |item| near.push(*item));
    assert_eq!(near, [[1_000.0, 2_000.0]]);
  }

  use super::*;
  use crate::terrain::channels::Fall;
  use vista_types::TerrainMetadata;

  fn fall_at(celsius: f32) -> RiverNetwork {
    RiverNetwork {
      falls: vec![Fall {
        lip: [32.0, 30.0],
        lip_level: 40.0,
        foot: [32.0, 32.0],
        foot_level: 20.0,
        direction: [0.0, 1.0],
        width: 4.0,
        discharge: 2.0,
        speed: 1.0,
        pool_radius: 10.0,
        pool_depth: 3.0,
        celsius,
        trickle: false,
        steps: Vec::new(),
      }],
      ..RiverNetwork::default()
    }
  }

  #[test]
  fn waterfalls_are_heard_unless_frozen() {
    let metadata = TerrainMetadata {
      metres_per_sample: 10.0,
      sea_level_metres: -100.0,
      ..TerrainMetadata::default()
    };
    let map = HeightMap::flat(64, 64, 20.0, metadata);
    let listener = [0.0, 25.0, 60.0];
    let open = SoundMap::build(&map, &fall_at(5.0)).query(listener, 1.0);
    let heard = open.waterfall.expect("a waterfall");

    assert!(heard.loudness > 0.5);
    assert!(open.river.is_none() && open.surf.is_none());
    assert!(SoundMap::build(&map, &fall_at(-10.0))
      .query(listener, 1.0)
      .waterfall
      .is_none());
  }
}
