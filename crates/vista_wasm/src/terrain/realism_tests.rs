//! Realism tests for the full CPU generation pipeline: every landform on
//! seeds 1 to 12, at 256 x 256 samples and 12 m per sample, with
//! `"high"` erosion. Every landform on seeds 1 and 2 at 512 x 512 and
//! 40 m per sample, with `"preview"` erosion, also runs through the same
//! checks: those maps are wide enough for a full 256-sample coarse grid,
//! which stage B solves coarse to fine.
//!
//! Slopes use central differences. Cells within [`BORDER`] samples of the
//! map edge are left out: the edge is a boundary condition (an outlet for
//! stream power and a sink for hydraulic erosion), not terrain.

use std::sync::OnceLock;

use vista_types::{ErosionOptions, ErosionQuality, FractalTerrainOptions, LandformKind};

use crate::terrain::drainage::{edge_or_sea_outlet, neighbours, priority_flood, NO_RECEIVER};
use crate::terrain::fractal::{generate_fractal_heightmap, MIN_BASIN_SAMPLES};
use crate::terrain::heightmap::HeightMap;
use crate::terrain::landforms::Landform;

const SIZE: u32 = 256;
const METRES_PER_SAMPLE: f32 = 12.0;
const SEEDS: std::ops::RangeInclusive<u64> = 1..=12;
const WIDE_SIZE: u32 = 512;
const WIDE_METRES_PER_SAMPLE: f32 = 40.0;
const WIDE_SEEDS: std::ops::RangeInclusive<u64> = 1..=2;
const BORDER: usize = 8;

const LANDFORMS: [LandformKind; 7] = [
  LandformKind::Continental,
  LandformKind::Alpine,
  LandformKind::RollingHills,
  LandformKind::Archipelago,
  LandformKind::MesaDesert,
  LandformKind::Fjords,
  LandformKind::VolcanicIsland,
];

fn options(landform: LandformKind, seed: u64) -> FractalTerrainOptions {
  FractalTerrainOptions {
    seed,
    size: SIZE,
    horizontal_scale_metres: METRES_PER_SAMPLE,
    landform,
    erosion: Some(ErosionOptions {
      quality: Some(ErosionQuality::High),
      ..ErosionOptions::default()
    }),
    ..FractalTerrainOptions::default()
  }
}

fn wide_options(landform: LandformKind, seed: u64) -> FractalTerrainOptions {
  FractalTerrainOptions {
    size: WIDE_SIZE,
    horizontal_scale_metres: WIDE_METRES_PER_SAMPLE,
    erosion: Some(ErosionOptions {
      quality: Some(ErosionQuality::Preview),
      ..ErosionOptions::default()
    }),
    ..options(landform, seed)
  }
}

struct Sample {
  landform: LandformKind,
  seed: u64,
  map: HeightMap,
}

/// Every landform and seed at both scales, generated once and shared by
/// the tests.
fn samples() -> &'static [Sample] {
  static SAMPLES: OnceLock<Vec<Sample>> = OnceLock::new();

  SAMPLES.get_or_init(|| {
    let mut jobs: Vec<FractalTerrainOptions> = LANDFORMS
      .iter()
      .flat_map(|landform| SEEDS.map(move |seed| options(*landform, seed)))
      .collect();
    jobs.extend(
      LANDFORMS
        .iter()
        .flat_map(|landform| WIDE_SEEDS.map(move |seed| wide_options(*landform, seed))),
    );
    let threads = std::thread::available_parallelism()
      .map_or(2, |n| n.get())
      .min(8);
    let chunk = jobs.len().div_ceil(threads);

    std::thread::scope(|scope| {
      let handles: Vec<_> = jobs
        .chunks(chunk)
        .map(|jobs| {
          scope.spawn(move || {
            jobs
              .iter()
              .map(|options| Sample {
                landform: options.landform,
                seed: options.seed,
                map: generate_fractal_heightmap(options).unwrap(),
              })
              .collect::<Vec<_>>()
          })
        })
        .collect();

      handles
        .into_iter()
        .flat_map(|handle| handle.join().unwrap())
        .collect()
    })
  })
}

/// Slope in degrees at an interior sample.
fn slope_degrees(map: &HeightMap, x: usize, y: usize) -> f32 {
  let n = map.metadata.width as usize;
  let h = &map.heights;
  let spacing = map.metadata.metres_per_sample;
  let dx = (h[y * n + x + 1] - h[y * n + x - 1]) / (2.0 * spacing);
  let dy = (h[(y + 1) * n + x] - h[(y - 1) * n + x]) / (2.0 * spacing);
  (dx * dx + dy * dy).sqrt().atan().to_degrees()
}

/// Interior land samples, as `(x, y, index)`.
fn land(map: &HeightMap) -> Vec<(usize, usize, usize)> {
  let n = map.metadata.width as usize;
  let sea = map.metadata.sea_level_metres;
  let mut cells = Vec::new();

  for y in BORDER..n - BORDER {
    for x in BORDER..n - BORDER {
      let i = y * n + x;

      if map.heights[i] > sea {
        cells.push((x, y, i));
      }
    }
  }

  cells
}

/// Closed depressions after priority-flood, as a component label per
/// sample (`usize::MAX` outside any) and each component's size.
fn depressions(map: &HeightMap) -> (Vec<usize>, Vec<usize>, Vec<u32>) {
  let n = map.metadata.width;
  let heights: Vec<f64> = map.heights.iter().map(|h| *h as f64).collect();
  let sea = map.metadata.sea_level_metres as f64;
  let flood = priority_flood(n, n, &heights, 0.0, edge_or_sea_outlet(n, n, &heights, sea));
  let mut label = vec![usize::MAX; heights.len()];
  let mut sizes = Vec::new();

  for start in 0..heights.len() {
    if label[start] != usize::MAX || flood.filled[start] <= heights[start] {
      continue;
    }

    let id = sizes.len();
    let mut stack = vec![start];
    let mut size = 0;
    label[start] = id;

    while let Some(cell) = stack.pop() {
      size += 1;

      for neighbour in neighbours(n, n, cell as u32) {
        let j = neighbour as usize;

        if label[j] == usize::MAX && flood.filled[j] > heights[j] {
          label[j] = id;
          stack.push(j);
        }
      }
    }

    sizes.push(size);
  }

  (label, sizes, flood.receiver)
}

fn describe(sample: &Sample) -> String {
  format!(
    "{:?} seed {} at {} m",
    sample.landform, sample.seed, sample.map.metadata.metres_per_sample
  )
}

/// Collects every failing map so one run reports them all.
#[derive(Default)]
struct Failures(Vec<String>);

impl Failures {
  fn check(&mut self, ok: bool, message: impl FnOnce() -> String) {
    if !ok {
      self.0.push(message());
    }
  }

  fn assert_none(self) {
    assert!(self.0.is_empty(), "{}", self.0.join("\n"));
  }
}

#[test]
fn maps_are_not_spiky() {
  let mut failures = Failures::default();

  for sample in samples() {
    let cells = land(&sample.map);
    let count = cells.len() as f32;
    let steeper = |angle: f32| {
      cells
        .iter()
        .filter(|(x, y, _)| slope_degrees(&sample.map, *x, *y) > angle)
        .count() as f32
        / count
    };
    let over_60_limit = match sample.landform {
      LandformKind::Alpine | LandformKind::Fjords => 0.03,
      _ => 0.005,
    };

    failures.check(steeper(45.0) < 0.05, || {
      format!("{}: {} over 45 degrees", describe(sample), steeper(45.0))
    });
    failures.check(steeper(60.0) < over_60_limit, || {
      format!("{}: {} over 60 degrees", describe(sample), steeper(60.0))
    });

    if sample.landform == LandformKind::RollingHills {
      failures.check(steeper(30.0) == 0.0, || {
        format!("{}: {} over 30 degrees", describe(sample), steeper(30.0))
      });
    }
  }

  failures.assert_none();
}

#[test]
fn peaks_are_few() {
  let mut failures = Failures::default();

  // The generator before landforms (ridged value noise at every octave,
  // no erosion) gave 142 to 169 strict local maxima per square kilometre
  // of land on these seeds at 256 x 256 and 12 m.
  for sample in samples() {
    let map = &sample.map;
    let n = map.metadata.width as usize;
    let cells = land(map);
    let peaks = cells
      .iter()
      .filter(|(x, y, i)| {
        let height = map.heights[*i];
        (-1..=1).all(|dy: isize| {
          (-1..=1).all(|dx: isize| {
            let j = (*y as isize + dy) as usize * n + (*x as isize + dx) as usize;
            (dx == 0 && dy == 0) || map.heights[j] < height
          })
        })
      })
      .count();
    let spacing = map.metadata.metres_per_sample;
    let area_km2 = cells.len() as f32 * spacing * spacing / 1_000_000.0;
    let density = peaks as f32 / area_km2;

    failures.check(density <= 4.0, || {
      format!("{}: {density} peaks per km2", describe(sample))
    });
  }

  failures.assert_none();
}

#[test]
fn land_drains_to_the_sea_or_a_lake() {
  let mut failures = Failures::default();

  for sample in samples() {
    let map = &sample.map;
    let (label, sizes, receiver) = depressions(map);
    let lake = |i: usize| label[i] != usize::MAX && sizes[label[i]] >= MIN_BASIN_SAMPLES;
    let cells = land(map);
    let drained = cells
      .iter()
      .filter(|(_, _, start)| {
        let mut cell = *start;

        loop {
          if lake(cell) {
            return true;
          }

          if label[cell] != usize::MAX {
            // A closed pit too small to be a lake.
            return false;
          }

          match receiver[cell] {
            NO_RECEIVER => return true,
            next => cell = next as usize,
          }
        }
      })
      .count();
    let fraction = drained as f32 / cells.len() as f32;

    failures.check(fraction >= 0.97, || {
      format!("{}: {fraction} drains", describe(sample))
    });
  }

  failures.assert_none();
}

#[test]
fn lowlands_are_smooth() {
  let mut failures = Failures::default();

  // Lowlands: level ground (land with slopes under 5 degrees at a sample
  // and all eight of its neighbours) in the lower half of the land by
  // height. Shore samples are left out, since their neighbours include
  // the sea floor.
  // Crest lines and summit ridges are level along their length but are
  // not lowlands. Lake beds lie under water and are left out. A map needs
  // 100 such samples to have lowlands at all; a volcanic island is slopes
  // from shore to rim.
  for sample in samples() {
    let map = &sample.map;
    let n = map.metadata.width as usize;
    let (label, sizes, _) = depressions(map);
    let lake = |i: usize| label[i] != usize::MAX && sizes[label[i]] >= MIN_BASIN_SAMPLES;
    let cells = land(map);
    let mut heights: Vec<f32> = cells.iter().map(|(_, _, i)| map.heights[*i]).collect();
    heights.sort_by(|a, b| a.total_cmp(b));
    let median = heights[heights.len() / 2];
    let level: Vec<usize> = cells
      .iter()
      .filter(|(x, y, i)| {
        !lake(*i)
          && map.heights[*i] <= median
          && (-1..=1).all(|dy: isize| {
            (-1..=1).all(|dx: isize| {
              let j = (*y as isize + dy) as usize * n + (*x as isize + dx) as usize;
              map.heights[j] > map.metadata.sea_level_metres
                && slope_degrees(
                  map,
                  (*x as isize + dx) as usize,
                  (*y as isize + dy) as usize,
                ) < 5.0
            })
          })
      })
      .map(|(_, _, i)| *i)
      .collect();

    if level.len() < 100 {
      continue;
    }

    let h = &map.heights;
    let mean = level
      .iter()
      .map(|i| (h[i - 1] + h[i + 1] + h[i - n] + h[i + n] - 4.0 * h[*i]).abs())
      .sum::<f32>()
      / level.len() as f32;

    failures.check(mean < 0.02 * map.metadata.metres_per_sample, || {
      format!("{}: mean |laplacian| {mean} m", describe(sample))
    });
  }

  failures.assert_none();
}

#[test]
fn land_fraction_matches_the_landform() {
  let mut failures = Failures::default();

  for sample in samples() {
    let map = &sample.map;
    let sea = map.metadata.sea_level_metres;
    let fraction =
      map.heights.iter().filter(|h| **h > sea).count() as f32 / map.heights.len() as f32;
    let target = Landform::preset(sample.landform).land_fraction;

    failures.check((fraction - target).abs() <= 0.05, || {
      format!("{}: land {fraction}", describe(sample))
    });
  }

  failures.assert_none();
}

#[test]
fn generation_is_bit_identical_per_seed() {
  let first = &samples()
    .iter()
    .find(|sample| sample.landform == LandformKind::Alpine && sample.seed == 3)
    .unwrap()
    .map;
  let again = generate_fractal_heightmap(&options(LandformKind::Alpine, 3)).unwrap();
  let same = first
    .heights
    .iter()
    .zip(&again.heights)
    .all(|(a, b)| a.to_bits() == b.to_bits());

  assert!(same);
  assert_eq!(first.aux, again.aux);
}

#[test]
fn landforms_differ() {
  let continental = generate_fractal_heightmap(&options(LandformKind::Continental, 1)).unwrap();
  let hills = generate_fractal_heightmap(&options(LandformKind::RollingHills, 1)).unwrap();

  assert!(continental.metadata.max_height_metres > hills.metadata.max_height_metres * 1.5);
  assert!(continental
    .aux
    .as_ref()
    .is_some_and(|aux| !aux.drainage_area.is_empty()));
}
