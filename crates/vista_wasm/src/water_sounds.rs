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

#[derive(Clone, Debug, Default)]
struct Grid {
  origin: [f32; 2],
  columns: i32,
  rows: i32,
  /// Where each cell's sources start in `sources`, plus one end.
  start: Vec<u32>,
  sources: Vec<Source>,
}

/// Water sound sources on a terrain.
#[derive(Clone, Debug, Default)]
pub struct SoundMap {
  grids: Vec<Grid>,
}

fn grid(kind: usize, half: [f32; 2], sources: Vec<Source>) -> Grid {
  let cell = RADIUS[kind];
  let origin = [-half[0], -half[1]];
  let columns = (half[0] * 2.0 / cell).ceil() as i32 + 1;
  let rows = (half[1] * 2.0 / cell).ceil() as i32 + 1;
  let slot = |s: &Source| {
    let x = (((s.position[0] - origin[0]) / cell) as i32).clamp(0, columns - 1);
    let y = (((s.position[2] - origin[1]) / cell) as i32).clamp(0, rows - 1);
    (y * columns + x) as usize
  };
  let mut start = vec![0u32; (columns * rows) as usize + 1];

  for source in &sources {
    start[slot(source) + 1] += 1;
  }

  for i in 1..start.len() {
    start[i] += start[i - 1];
  }

  let mut next = start.clone();
  let mut sorted = vec![Source::default(); sources.len()];

  for source in sources {
    let at = &mut next[slot(&source)];
    sorted[*at as usize] = source;
    *at += 1;
  }

  Grid {
    origin,
    columns,
    rows,
    start,
    sources: sorted,
  }
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

    for reach in &rivers.reaches {
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
      let cx = ((listener[0] - grid.origin[0]) / cell).floor() as i32;
      let cy = ((listener[2] - grid.origin[1]) / cell).floor() as i32;
      let scale = if kind == 3 { wave_height.max(0.0) } else { 1.0 };

      for y in (cy - 1).max(0)..=(cy + 1).min(grid.rows - 1) {
        for x in (cx - 1).max(0)..=(cx + 1).min(grid.columns - 1) {
          let slot = (y * grid.columns + x) as usize;

          for source in &grid.sources[grid.start[slot] as usize..grid.start[slot + 1] as usize] {
            let d = [
              source.position[0] - listener[0],
              source.position[1] - listener[1],
              source.position[2] - listener[2],
            ];
            let squared = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];

            if squared > cell * cell {
              continue;
            }

            let strength = source.strength * scale * HALF_LOUD[kind];
            let loudness = strength / (strength + squared).max(1e-6);

            if found[kind]
              .as_ref()
              .is_none_or(|best| loudness > best.loudness)
            {
              found[kind] = Some(WaterSound {
                distance_metres: squared.sqrt(),
                loudness,
                position: source.position,
              });
            }
          }
        }
      }
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
