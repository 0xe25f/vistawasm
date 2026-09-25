//! Hydrology: where water comes from and where it goes.
//!
//! Water is routed over the finished heightmap (after erosion, glacier
//! shaping and any painted water mask) on a flow grid that is the full
//! heightmap resolution up to [`MAX_GRID`] samples per side. Every cell
//! adds its runoff (precipitation from the climate's moisture, plus
//! snowmelt), which is accumulated downstream as a mean discharge in
//! cubic metres per second. Depressions fill into lakes that overflow at
//! their spill point, or keep their water when evaporation takes all of
//! it. Channels are the cells whose discharge passes a threshold, plus
//! everything downstream of explicit sources: glacier snouts, the lower
//! edge of snow fields, springs at the foot of slopes, and the outlets of
//! lakes fed by a channel.

use vista_types::{BiomeKind, RiverOptions};

use crate::maths::{hash_u64, length2};
use crate::terrain::biomes::{SurfaceSample, MAT_SNOW};
use crate::terrain::drainage::{self, NO_RECEIVER};
use crate::terrain::heightmap::HeightMap;

/// Largest flow grid, in samples per side. Heightmaps up to this size are
/// routed at full resolution; larger ones every second (or fourth, ...)
/// sample, which keeps memory and build time bounded.
pub const MAX_GRID: u32 = 1024;

/// Seconds in a mean year.
pub const SECONDS_PER_YEAR: f32 = 31_557_600.0;

/// Discharge per square kilometre that turns `minCatchmentKm2` into a
/// discharge threshold: about 950 mm of runoff a year.
pub const DISCHARGE_PER_KM2: f32 = 0.03;

/// Discharge of one spring, in cubic metres per second.
pub const SPRING_DISCHARGE: f32 = 0.02;

/// Springs are never closer together than this.
pub const SPRING_SPACING_METRES: f32 = 600.0;

/// Snow melts off at this depth of water a year, per unit of snow and of
/// `snowmelt`.
const SNOWMELT_METRES_PER_YEAR: f32 = 0.6;

/// A depression with at least this many flow cells, or at least
/// [`MIN_LAKE_DEPTH_METRES`] deep, becomes a lake.
pub const MIN_LAKE_CELLS: usize = 24;

/// See [`MIN_LAKE_CELLS`].
pub const MIN_LAKE_DEPTH_METRES: f32 = 1.5;

/// Snow fields smaller than this many cells are patches, not sources.
const MIN_SNOW_FIELD_CELLS: usize = 8;

/// Marks a cell outside every lake.
pub const NO_LAKE: u32 = u32::MAX;

/// A lake filling a depression to its spill height.
#[derive(Clone, Debug, PartialEq)]
pub struct Lake {
  /// Flow cells under the lake.
  pub cells: Vec<u32>,
  /// Water surface height in metres: the spill height.
  pub surface: f32,
  /// Deepest point below the surface, in metres.
  pub depth: f32,
  /// The lake cell all of its water leaves through.
  pub exit: u32,
  /// The cell just outside the lake where its outlet river starts.
  pub outlet: u32,
  /// Mean inflow, in cubic metres per second, before evaporation.
  pub inflow: f32,
  /// Mean evaporation from the surface, in cubic metres per second.
  pub evaporation: f32,
  /// Whether evaporation takes all of the inflow, so the lake has no
  /// outlet.
  pub endorheic: bool,
  /// Mean annual temperature at the outlet, in °C.
  pub celsius: f32,
}

/// Where a stream ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mouth {
  /// It reaches the sea; the last cell is a sea cell.
  Sea,
  /// It enters a lake; the last cell is a lake cell.
  Lake(u32),
  /// It leaves the map.
  Edge,
  /// It joins a larger stream; the last cell belongs to that stream.
  Join,
  /// It runs on under glacier ice.
  Ice,
}

/// One stream: a run of channel cells from a head or confluence to its
/// mouth, following the flow.
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
  /// Flow cells from upstream to downstream, including the mouth cell
  /// for [`Mouth::Sea`], [`Mouth::Lake`] and [`Mouth::Join`].
  pub cells: Vec<u32>,
  /// Where it ends.
  pub mouth: Mouth,
}

/// Water routed over a heightmap.
#[derive(Clone, Debug, Default)]
pub struct Hydrology {
  /// Flow grid width in cells.
  pub width: u32,
  /// Flow grid height in cells.
  pub height: u32,
  /// Heightmap samples per flow cell along each axis.
  pub stride: u32,
  /// Width of a flow cell in metres.
  pub cell_metres: f32,
  /// Sea level in metres.
  pub sea: f32,
  /// Ground height of each cell.
  pub ground: Vec<f32>,
  /// Heights with every depression filled to its spill height: the water
  /// level a channel or lake has there.
  pub filled: Vec<f32>,
  /// The cell each cell drains into, or [`NO_RECEIVER`] for outlets.
  pub receiver: Vec<u32>,
  /// Mean discharge in cubic metres per second.
  pub discharge: Vec<f32>,
  /// The lake each cell lies in, or [`NO_LAKE`].
  pub lake: Vec<u32>,
  /// The lakes.
  pub lakes: Vec<Lake>,
  /// Whether each cell carries a channel.
  pub channel: Vec<bool>,
  /// Whether each cell is glacier ice.
  pub glacier: Vec<bool>,
  /// Spring cells.
  pub springs: Vec<u32>,
  /// Explicit sources: glacier snouts, snow-field edges and springs.
  pub sources: Vec<u32>,
}

impl Hydrology {
  /// The heightmap sample index of a flow cell.
  pub fn sample_index(&self, cell: u32, map_width: u32) -> usize {
    let (x, y) = self.sample_xy(cell);
    (y * map_width + x) as usize
  }

  /// The heightmap sample coordinates of a flow cell.
  pub fn sample_xy(&self, cell: u32) -> (u32, u32) {
    (
      (cell % self.width) * self.stride,
      (cell / self.width) * self.stride,
    )
  }

  /// Whether a cell is sea.
  pub fn is_sea(&self, cell: u32) -> bool {
    self.ground[cell as usize] <= self.sea
  }

  /// Whether a channel cell is drawn: channels under glacier ice are not.
  pub fn is_drawn(&self, cell: u32) -> bool {
    self.channel[cell as usize] && !self.glacier[cell as usize]
  }

  /// Split the drawn channel network into streams. At each confluence the
  /// tributary with the larger discharge carries on, and the others end
  /// there, so main stems stay whole. Every stream comes before the
  /// streams that join it.
  pub fn streams(&self) -> Vec<Stream> {
    let count = self.ground.len();
    let mut main_donor = vec![NO_RECEIVER; count];

    for cell in 0..count as u32 {
      let r = self.receiver[cell as usize];

      if !self.is_drawn(cell) || r == NO_RECEIVER || !self.is_drawn(r) {
        continue;
      }

      let current = main_donor[r as usize];

      if current == NO_RECEIVER || self.discharge[cell as usize] > self.discharge[current as usize]
      {
        main_donor[r as usize] = cell;
      }
    }

    let mut streams = Vec::new();

    for head in 0..count as u32 {
      if !self.is_drawn(head) || main_donor[head as usize] != NO_RECEIVER {
        continue;
      }

      let mut cells = vec![head];
      let mut cell = head;
      let mouth = loop {
        let r = self.receiver[cell as usize];

        if r == NO_RECEIVER {
          break Mouth::Edge;
        }

        if !self.is_drawn(r) {
          if self.lake[r as usize] != NO_LAKE {
            cells.push(r);
            break Mouth::Lake(self.lake[r as usize]);
          }

          if self.is_sea(r) {
            cells.push(r);
            break Mouth::Sea;
          }

          break Mouth::Ice;
        }

        if main_donor[r as usize] != cell {
          cells.push(r);
          break Mouth::Join;
        }

        cells.push(r);
        cell = r;
      };

      streams.push(Stream { cells, mouth });
    }

    // Emit streams in the drainage's stack order of their last cell, which
    // puts every receiver before its donors: a stream comes before the
    // tributaries that join it.
    let mut ending = vec![NO_RECEIVER; count];
    let mut next = vec![NO_RECEIVER; streams.len()];

    for (index, stream) in streams.iter().enumerate() {
      let last = *stream.cells.last().unwrap_or(&0) as usize;
      next[index] = ending[last];
      ending[last] = index as u32;
    }

    let mut taken: Vec<Option<Stream>> = streams.into_iter().map(Some).collect();
    let mut ordered = Vec::with_capacity(taken.len());

    for cell in drainage::stack_order(&self.receiver) {
      let mut index = ending[cell as usize];

      while index != NO_RECEIVER {
        if let Some(stream) = taken[index as usize].take() {
          ordered.push(stream);
        }

        index = next[index as usize];
      }
    }

    ordered
  }
}

/// How wet a year is, in metres of precipitation, from climate moisture
/// (0 to 1): 300 mm in the driest climates to 3000 mm in the wettest.
pub fn precipitation_metres(moisture: f32) -> f32 {
  0.3 + moisture.clamp(0.0, 1.0) * 2.7
}

/// How much snow lies on a sample, 0 to 1, for snowmelt.
pub fn snow_amount(sample: &SurfaceSample) -> f32 {
  let biome = match sample.biome_kind() {
    BiomeKind::UpperSnowyPeaks => 1.0,
    BiomeKind::LowerSnowyPeaks => 0.6,
    _ => 0.0,
  };

  sample
    .permanent_snow_unit()
    .max(sample.weight(MAT_SNOW))
    .max(biome)
}

/// Mean evaporation from open water, in metres a year: more where it is
/// warm and dry.
pub fn evaporation_metres(celsius: f32, moisture: f32) -> f32 {
  (0.25 + 0.055 * celsius).clamp(0.05, 2.5) * (1.4 - moisture).clamp(0.2, 1.4)
}

/// The discharge above which a cell is a channel, from
/// `minCatchmentKm2`.
pub fn discharge_threshold(options: &RiverOptions) -> f32 {
  options.min_catchment_km2.max(0.0) * DISCHARGE_PER_KM2
}

/// Route water over `map`. `surface` holds the map's surface samples
/// before any river was carved (or is empty, for a plain climate), and
/// `seed` places springs.
pub fn build_hydrology(
  map: &HeightMap,
  surface: &[SurfaceSample],
  options: &RiverOptions,
  seed: u64,
) -> Hydrology {
  let map_width = map.metadata.width;
  let map_height = map.metadata.height;

  if map_width < 2 || map_height < 2 {
    return Hydrology::default();
  }

  let stride = (map_width.max(map_height).saturating_sub(1) / (MAX_GRID - 1)).max(1);
  let width = (map_width - 1) / stride + 1;
  let height = (map_height - 1) / stride + 1;
  let count = (width * height) as usize;
  let sea = map.metadata.sea_level_metres;
  let cell_metres = map.metadata.metres_per_sample.max(0.001) * stride as f32;
  let cell_area = cell_metres * cell_metres;
  let sample = |cell: usize| {
    let x = (cell as u32 % width) * stride;
    let y = (cell as u32 / width) * stride;
    (y * map_width + x) as usize
  };
  let climate = |cell: usize| surface.get(sample(cell));
  let mut ground64 = Vec::with_capacity(count);

  for cell in 0..count {
    let index = sample(cell);
    ground64.push(if map.no_data[index] {
      sea as f64 - 1.0
    } else {
      map.heights[index] as f64
    });
  }

  // No gradient across filled flats: lakes need their exact spill height.
  // Flats still drain along the flood's own receivers.
  let flood = drainage::priority_flood(
    width,
    height,
    &ground64,
    0.0,
    drainage::edge_or_sea_outlet(width, height, &ground64, sea as f64),
  );
  let mut receiver = flood.receiver;
  drainage::steepest_receivers(width, height, &flood.filled, &mut receiver);
  let ground: Vec<f32> = ground64.iter().map(|h| *h as f32).collect();
  let filled: Vec<f32> = flood.filled.iter().map(|h| *h as f32).collect();
  let glacier: Vec<bool> = (0..count)
    .map(|cell| climate(cell).is_some_and(|s| s.is_glacier()))
    .collect();

  let mut hydrology = Hydrology {
    width,
    height,
    stride,
    cell_metres,
    sea,
    ground,
    filled,
    receiver: Vec::new(),
    discharge: vec![0.0; count],
    lake: vec![NO_LAKE; count],
    lakes: Vec::new(),
    channel: vec![false; count],
    glacier,
    springs: Vec::new(),
    sources: Vec::new(),
  };
  find_lakes(&mut hydrology, &flood.filled, &ground64, &mut receiver);
  hydrology.receiver = receiver;
  let order = drainage::stack_order(&hydrology.receiver);
  let area_cells = drainage::accumulate(&order, &hydrology.receiver, vec![1.0; count]);

  // Runoff from rain and snowmelt, in cubic metres per second.
  let snowmelt = options.snowmelt.clamp(0.0, 2.0);

  for cell in 0..count {
    if hydrology.is_sea(cell as u32) {
      continue;
    }

    let (moisture, snow) =
      climate(cell).map_or((0.5, 0.0), |s| (s.moisture_unit(), snow_amount(s)));
    let metres = precipitation_metres(moisture) + snowmelt * snow * SNOWMELT_METRES_PER_YEAR;
    hydrology.discharge[cell] = metres * cell_area / SECONDS_PER_YEAR;
  }

  if options.springs {
    hydrology.springs = find_springs(&hydrology, &area_cells, seed);

    for spring in &hydrology.springs {
      hydrology.discharge[*spring as usize] += SPRING_DISCHARGE;
    }
  }

  for lake in &mut hydrology.lakes {
    let exit = lake.exit as usize;
    let moisture = climate(exit).map_or(0.5, |s| s.moisture_unit());
    lake.celsius = climate(lake.outlet as usize).map_or(15.0, |s| s.celsius());
    lake.evaporation =
      evaporation_metres(lake.celsius, moisture) * lake.cells.len() as f32 * cell_area
        / SECONDS_PER_YEAR;
  }

  // Accumulate downstream. All of a lake's water leaves through its exit
  // cell, where evaporation from the lake surface is taken off.
  for cell in order.iter().rev() {
    let i = *cell as usize;
    let r = hydrology.receiver[i];

    if r == NO_RECEIVER {
      continue;
    }

    let mut passing = hydrology.discharge[i];
    let lake = hydrology.lake[i];

    if lake != NO_LAKE && hydrology.lakes[lake as usize].exit == *cell {
      let lake = &mut hydrology.lakes[lake as usize];
      lake.inflow = passing;
      passing = (passing - lake.evaporation).max(0.0);
      lake.endorheic = passing <= 0.0;
    }

    hydrology.discharge[r as usize] += passing;
  }

  if snowmelt > 0.0 {
    hydrology.sources.extend(glacier_snouts(&hydrology));
    hydrology
      .sources
      .extend(snow_field_edges(&hydrology, surface, map_width));
  }

  hydrology.sources.extend(hydrology.springs.iter().copied());
  mark_channels(&mut hydrology, discharge_threshold(options));
  hydrology
}

/// Label lakes: connected depressed cells with at least
/// [`MIN_LAKE_CELLS`] cells or [`MIN_LAKE_DEPTH_METRES`] deep. Inside each,
/// water is re-routed to flow to one exit cell, so evaporation can be
/// taken off in one place.
fn find_lakes(hydrology: &mut Hydrology, filled: &[f64], ground: &[f64], receiver: &mut [u32]) {
  let (width, height) = (hydrology.width, hydrology.height);
  let count = filled.len();
  let sea = hydrology.sea as f64;
  let depressed = |i: usize| filled[i] - ground[i] > 1e-4 && ground[i] > sea;
  let mut label = vec![u32::MAX; count];
  let mut stack = Vec::new();
  let mut seen = vec![false; count];
  let mut queue = Vec::new();

  for start in 0..count {
    if label[start] != u32::MAX || !depressed(start) {
      continue;
    }

    let id = hydrology.lakes.len() as u32;
    let mut cells = Vec::new();
    let mut deepest = 0.0f64;
    label[start] = id;
    stack.push(start as u32);

    while let Some(cell) = stack.pop() {
      cells.push(cell);
      deepest = deepest.max(filled[cell as usize] - ground[cell as usize]);

      for n in drainage::neighbours(width, height, cell) {
        if label[n as usize] == u32::MAX && depressed(n as usize) {
          label[n as usize] = id;
          stack.push(n);
        }
      }
    }

    if cells.len() < MIN_LAKE_CELLS && deepest < MIN_LAKE_DEPTH_METRES as f64 {
      // Too small for a lake: a hollow the water just runs through. Mark
      // it so it is not visited again.
      for cell in &cells {
        label[*cell as usize] = u32::MAX - 1;
      }

      continue;
    }

    // The exit is the lake cell draining to the lowest cell outside.
    let mut exit = cells[0];
    let mut best = f64::INFINITY;

    for cell in &cells {
      let r = receiver[*cell as usize];

      if r != NO_RECEIVER && label[r as usize] != id {
        let level = filled[r as usize];

        if level < best || (level == best && *cell < exit) {
          best = level;
          exit = *cell;
        }
      }
    }

    // Breadth-first from the exit, so every lake cell drains to it.
    queue.clear();
    queue.push(exit);
    seen[exit as usize] = true;
    let mut head = 0;

    while head < queue.len() {
      let cell = queue[head];
      head += 1;

      for n in drainage::neighbours(width, height, cell) {
        if label[n as usize] == id && !seen[n as usize] {
          seen[n as usize] = true;
          receiver[n as usize] = cell;
          queue.push(n);
        }
      }
    }

    for cell in &cells {
      hydrology.lake[*cell as usize] = id;
    }

    hydrology.lakes.push(Lake {
      surface: filled[exit as usize] as f32,
      depth: deepest as f32,
      exit,
      outlet: receiver[exit as usize],
      cells,
      inflow: 0.0,
      evaporation: 0.0,
      endorheic: false,
      celsius: 15.0,
    });
  }
}

/// Gradient (rise over run) at each cell, from central differences.
fn gradients(hydrology: &Hydrology) -> Vec<f32> {
  let (width, height) = (hydrology.width as i32, hydrology.height as i32);
  let at = |x: i32, y: i32| {
    hydrology.ground[(y.clamp(0, height - 1) * width + x.clamp(0, width - 1)) as usize]
  };
  let mut slope = Vec::with_capacity(hydrology.ground.len());

  for y in 0..height {
    for x in 0..width {
      let dx = (at(x + 1, y) - at(x - 1, y)) / (2.0 * hydrology.cell_metres);
      let dy = (at(x, y + 1) - at(x, y - 1)) / (2.0 * hydrology.cell_metres);
      slope.push(length2(dx, dy));
    }
  }

  slope
}

/// Springs at the foot of slopes: concave cells gentler than 8 degrees
/// just below ground steeper than 20 degrees, draining more than
/// 0.05 km^2. The seed picks one candidate per 300 m, and they are kept
/// at least [`SPRING_SPACING_METRES`] apart.
pub fn find_springs(hydrology: &Hydrology, area_cells: &[f32], seed: u64) -> Vec<u32> {
  let (width, height) = (hydrology.width as i32, hydrology.height as i32);
  let slope = gradients(hydrology);
  // tan(8 degrees) and tan(20 degrees).
  let (gentle, steep) = (0.1405, 0.364);
  let cell_km2 = hydrology.cell_metres * hydrology.cell_metres / 1.0e6;
  // The best candidate (smallest seeded hash) in each 300 m bucket.
  let bucket_metres = SPRING_SPACING_METRES * 0.5;
  let buckets_x = ((width as f32 * hydrology.cell_metres) / bucket_metres).ceil() as i32 + 1;
  let buckets_y = ((height as f32 * hydrology.cell_metres) / bucket_metres).ceil() as i32 + 1;
  let mut best = vec![(u64::MAX, NO_RECEIVER); (buckets_x * buckets_y) as usize];
  let position = |cell: u32| {
    (
      (cell % hydrology.width) as f32 * hydrology.cell_metres,
      (cell / hydrology.width) as f32 * hydrology.cell_metres,
    )
  };
  let bucket = |x: f32, y: f32| ((y / bucket_metres) as i32, (x / bucket_metres) as i32);

  for y in 1..height - 1 {
    for x in 1..width - 1 {
      let cell = (y * width + x) as usize;
      let h = hydrology.ground[cell];

      if hydrology.is_sea(cell as u32)
        || hydrology.lake[cell] != NO_LAKE
        || hydrology.glacier[cell]
        || slope[cell] >= gentle
        || area_cells[cell] * cell_km2 <= 0.05
      {
        continue;
      }

      let at = |dx: i32, dy: i32| hydrology.ground[((y + dy) * width + x + dx) as usize];
      let concave = (at(-1, 0) + at(1, 0) + at(0, -1) + at(0, 1)) * 0.25 > h;
      let mut steep_above = false;

      for dy in -2..=2 {
        for dx in -2..=2 {
          let (nx, ny) = (x + dx, y + dy);

          if nx >= 0 && ny >= 0 && nx < width && ny < height {
            let n = (ny * width + nx) as usize;
            steep_above |= hydrology.ground[n] > h && slope[n] > steep;
          }
        }
      }

      if concave && steep_above {
        let key = hash_u64(seed ^ (cell as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let (px, py) = position(cell as u32);
        let (by, bx) = bucket(px, py);
        let slot = &mut best[(by * buckets_x + bx) as usize];

        if key < slot.0 {
          *slot = (key, cell as u32);
        }
      }
    }
  }

  // Then keep them in bucket order, dropping any within the spacing of
  // one already kept.
  let mut kept = vec![NO_RECEIVER; best.len()];
  let mut springs = Vec::new();

  for (index, (_, cell)) in best.iter().enumerate() {
    if *cell == NO_RECEIVER {
      continue;
    }

    let (px, py) = position(*cell);
    let (by, bx) = (index as i32 / buckets_x, index as i32 % buckets_x);
    let mut clear = true;

    for oy in -2..=2 {
      for ox in -2..=2 {
        let (nx, ny) = (bx + ox, by + oy);

        if nx >= 0 && ny >= 0 && nx < buckets_x && ny < buckets_y {
          let other = kept[(ny * buckets_x + nx) as usize];

          if other != NO_RECEIVER {
            let (qx, qy) = position(other);
            clear &= length2(qx - px, qy - py) >= SPRING_SPACING_METRES;
          }
        }
      }
    }

    if clear {
      kept[index] = *cell;
      springs.push(*cell);
    }
  }

  springs
}

/// Connected regions of cells where `inside` holds, with at least
/// `min_cells` cells, each reduced to its lowest cell.
fn lowest_of_regions(
  hydrology: &Hydrology,
  inside: impl Fn(u32) -> bool,
  min_cells: usize,
) -> Vec<u32> {
  let count = hydrology.ground.len();
  let mut seen = vec![false; count];
  let mut lowest = Vec::new();
  let mut stack = Vec::new();

  for start in 0..count as u32 {
    if seen[start as usize] || !inside(start) {
      continue;
    }

    seen[start as usize] = true;
    stack.push(start);
    let mut size = 0;
    let mut best = start;

    while let Some(cell) = stack.pop() {
      size += 1;
      let (h, b) = (
        hydrology.ground[cell as usize],
        hydrology.ground[best as usize],
      );

      if h < b || (h == b && cell < best) {
        best = cell;
      }

      for n in drainage::neighbours(hydrology.width, hydrology.height, cell) {
        if !seen[n as usize] && inside(n) {
          seen[n as usize] = true;
          stack.push(n);
        }
      }
    }

    if size >= min_cells {
      lowest.push(best);
    }
  }

  lowest
}

/// The snout of each glacier: meltwater runs on beneath the ice from its
/// lowest cell, and the stream starts at the first cell clear of it.
pub fn glacier_snouts(hydrology: &Hydrology) -> Vec<u32> {
  let mut snouts = Vec::new();

  for lowest in lowest_of_regions(hydrology, |cell| hydrology.glacier[cell as usize], 1) {
    let mut cell = lowest;

    while hydrology.glacier[cell as usize] {
      let r = hydrology.receiver[cell as usize];

      if r == NO_RECEIVER {
        break;
      }

      cell = r;
    }

    if !hydrology.glacier[cell as usize]
      && !hydrology.is_sea(cell)
      && hydrology.lake[cell as usize] == NO_LAKE
    {
      snouts.push(cell);
    }
  }

  snouts
}

/// The lowest cell of each snowy-peak region, where its meltwater
/// gathers.
fn snow_field_edges(hydrology: &Hydrology, surface: &[SurfaceSample], map_width: u32) -> Vec<u32> {
  if surface.is_empty() {
    return Vec::new();
  }

  let snowy = |cell: u32| {
    let sample = &surface[hydrology.sample_index(cell, map_width)];
    !hydrology.glacier[cell as usize]
      && matches!(
        sample.biome_kind(),
        BiomeKind::UpperSnowyPeaks | BiomeKind::LowerSnowyPeaks
      )
  };

  lowest_of_regions(hydrology, snowy, MIN_SNOW_FIELD_CELLS)
    .into_iter()
    .filter(|cell| hydrology.lake[*cell as usize] == NO_LAKE && !hydrology.is_sea(*cell))
    .collect()
}

/// Mark channel cells: land outside lakes whose discharge reaches
/// `threshold`, and everything downstream of an explicit source or of the
/// outlet of a lake that a channel flows into.
fn mark_channels(hydrology: &mut Hydrology, threshold: f32) {
  let count = hydrology.ground.len();
  let land = |h: &Hydrology, cell: u32| !h.is_sea(cell) && h.lake[cell as usize] == NO_LAKE;

  for cell in 0..count as u32 {
    hydrology.channel[cell as usize] =
      land(hydrology, cell) && hydrology.discharge[cell as usize] >= threshold;
  }

  let force = |hydrology: &mut Hydrology, start: u32| {
    let mut cell = start;

    while cell != NO_RECEIVER && land(hydrology, cell) && !hydrology.channel[cell as usize] {
      hydrology.channel[cell as usize] = true;
      cell = hydrology.receiver[cell as usize];
    }
  };

  for source in hydrology.sources.clone() {
    force(hydrology, source);
  }

  // A lake fed by a channel overflows into a channel, which may feed the
  // next lake down, so repeat until nothing changes.
  loop {
    let mut fed = vec![false; hydrology.lakes.len()];

    for cell in 0..count {
      let r = hydrology.receiver[cell];

      if hydrology.channel[cell] && r != NO_RECEIVER && hydrology.lake[r as usize] != NO_LAKE {
        fed[hydrology.lake[r as usize] as usize] = true;
      }
    }

    let mut changed = false;

    for (id, fed) in fed.into_iter().enumerate() {
      let lake = &hydrology.lakes[id];

      if fed
        && !lake.endorheic
        && lake.outlet != NO_RECEIVER
        && !hydrology.channel[lake.outlet as usize]
        && land(hydrology, lake.outlet)
      {
        let outlet = lake.outlet;
        force(hydrology, outlet);
        changed = true;
      }
    }

    if !changed {
      break;
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::terrain::heightmap::update_stats;
  use vista_types::TerrainMetadata;

  pub(crate) fn map_from(size: u32, metres: f32, height: impl Fn(u32, u32) -> f32) -> HeightMap {
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: metres,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 0.0, metadata);

    for y in 0..size {
      for x in 0..size {
        map.heights[(y * size + x) as usize] = height(x, y);
      }
    }

    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    map
  }

  fn options() -> RiverOptions {
    RiverOptions {
      min_catchment_km2: 0.5,
      ..RiverOptions::default()
    }
  }

  /// A valley draining north to the sea, with a bowl in its middle.
  fn valley_with_bowl() -> HeightMap {
    map_from(96, 40.0, |x, y| {
      let across = (x as f32 - 48.0).abs() * 6.0;
      let along = y as f32 * 4.0 - 20.0;
      let dx = x as f32 - 48.0;
      let dy = y as f32 - 50.0;
      let bowl = (12.0 - (dx * dx + dy * dy).sqrt()).max(0.0) * 3.0;
      along + across - bowl
    })
  }

  #[test]
  fn discharge_grows_with_rain_and_reaches_the_sea() {
    let map = valley_with_bowl();
    let wet = SurfaceSample {
      moisture: 255,
      ..SurfaceSample::default()
    };
    let dry = SurfaceSample::default();
    let wet_surface = vec![wet; map.heights.len()];
    let dry_surface = vec![dry; map.heights.len()];
    let wet_h = build_hydrology(&map, &wet_surface, &options(), 1);
    let dry_h = build_hydrology(&map, &dry_surface, &options(), 1);
    let total = |h: &Hydrology| -> f32 {
      (0..h.ground.len())
        .filter(|c| h.receiver[*c] != NO_RECEIVER && h.is_sea(h.receiver[*c]))
        .map(|c| h.discharge[c])
        .sum()
    };

    assert!(total(&wet_h) > total(&dry_h) * 5.0);
    assert!(
      wet_h.channel.iter().filter(|c| **c).count() > dry_h.channel.iter().filter(|c| **c).count()
    );
  }

  #[test]
  fn every_channel_ends_at_the_sea_a_lake_or_the_edge() {
    let map = valley_with_bowl();
    let hydrology = build_hydrology(&map, &[], &options(), 7);
    assert!(hydrology.channel.iter().any(|c| *c));

    for start in 0..hydrology.ground.len() as u32 {
      if !hydrology.channel[start as usize] {
        continue;
      }

      let mut cell = start;
      let mut steps = 0;

      loop {
        let r = hydrology.receiver[cell as usize];

        if r == NO_RECEIVER || hydrology.is_sea(r) || hydrology.lake[r as usize] != NO_LAKE {
          break;
        }

        assert!(
          hydrology.channel[r as usize],
          "channel {start} stops at {r}"
        );
        cell = r;
        steps += 1;
        assert!(steps < 100_000);
      }
    }

    let streams = hydrology.streams();
    assert!(streams.iter().all(|s| s.mouth != Mouth::Ice));
    let covered: usize = streams
      .iter()
      .map(|s| s.cells.iter().filter(|c| hydrology.is_drawn(**c)).count())
      .sum::<usize>()
      - streams.iter().filter(|s| s.mouth == Mouth::Join).count();
    assert_eq!(covered, hydrology.channel.iter().filter(|c| **c).count());
  }

  #[test]
  fn a_bowl_fills_to_its_spill_height_and_overflows_at_the_lowest_rim() {
    // A 30 m rim around a bowl, with a notch at 22 m on the east side.
    let map = map_from(64, 20.0, |x, y| {
      let dx = x as f32 - 32.0;
      let dy = y as f32 - 32.0;
      let r = (dx * dx + dy * dy).sqrt();
      let rim = if r < 14.0 {
        10.0 + r * 0.5
      } else {
        30.0 - (r - 14.0) * 1.2
      };
      if r > 12.0 && dx > 0.0 && dy.abs() < 1.5 {
        rim.min(22.0 - (r - 14.0).max(0.0) * 0.3)
      } else {
        rim
      }
    });
    let hydrology = build_hydrology(&map, &[], &options(), 3);
    let lake = hydrology
      .lakes
      .iter()
      .max_by_key(|lake| lake.cells.len())
      .expect("a lake");

    assert!(
      (lake.surface - 22.0).abs() < 0.05,
      "surface {}",
      lake.surface
    );
    let (ox, oy) = hydrology.sample_xy(lake.outlet);
    assert!(
      ox > 32 && (oy as i32 - 32).abs() <= 2,
      "outlet at {ox}, {oy}"
    );
    assert!(!lake.endorheic);
    assert!(lake.inflow > 0.0);
  }

  #[test]
  fn an_arid_basin_keeps_its_lake_without_an_outlet() {
    // A wide, shallow basin with almost no catchment beyond its shores.
    let map = map_from(64, 50.0, |x, y| {
      let dx = x as f32 - 32.0;
      let dy = y as f32 - 32.0;
      let r = (dx * dx + dy * dy).sqrt();
      if r < 24.0 {
        20.0
      } else if r < 26.0 {
        24.0
      } else {
        24.0 - (r - 26.0) * 2.0
      }
    });
    let hot_desert = SurfaceSample {
      moisture: 0,
      celsius_hundredths: 3000,
      ..SurfaceSample::default()
    };
    let hydrology = build_hydrology(&map, &vec![hot_desert; map.heights.len()], &options(), 3);
    let lake = &hydrology.lakes[0];

    assert!(lake.endorheic);
    assert!(!hydrology.channel[lake.outlet as usize]);
  }

  #[test]
  fn springs_are_seeded_and_spaced() {
    // Steep hills over a gentle plain, so slope feet are everywhere.
    let map = map_from(160, 20.0, |x, y| {
      let ridge = ((x as f32 * 0.21).sin() * (y as f32 * 0.17).cos()).max(0.0) * 60.0;
      50.0 + y as f32 * 0.4 + ridge
    });
    let first = build_hydrology(&map, &[], &options(), 11);
    let again = build_hydrology(&map, &[], &options(), 11);
    let other = build_hydrology(&map, &[], &options(), 12);

    assert!(!first.springs.is_empty());
    assert_eq!(first.springs, again.springs);
    assert_ne!(first.springs, other.springs);

    for (i, a) in first.springs.iter().enumerate() {
      for b in &first.springs[i + 1..] {
        let (ax, ay) = first.sample_xy(*a);
        let (bx, by) = first.sample_xy(*b);
        let distance = length2(ax as f32 - bx as f32, ay as f32 - by as f32) * 20.0;
        assert!(
          distance >= SPRING_SPACING_METRES,
          "springs {distance} m apart"
        );
      }
    }

    let none = build_hydrology(
      &map,
      &[],
      &RiverOptions {
        springs: false,
        ..options()
      },
      11,
    );
    assert!(none.springs.is_empty());
  }

  #[test]
  fn a_glacier_snout_starts_a_stream() {
    // A slope rising south, with glacier ice above y = 40.
    let map = map_from(64, 30.0, |x, y| {
      5.0 + y as f32 * 6.0 + (x as f32 - 32.0).abs() * 0.5
    });
    let surface: Vec<SurfaceSample> = (0..map.heights.len())
      .map(|i| {
        if i / 64 >= 40 {
          SurfaceSample {
            biome: BiomeKind::IceArctic as u8,
            permanent_snow: 255,
            ..SurfaceSample::default()
          }
        } else {
          SurfaceSample::default()
        }
      })
      .collect();
    // A threshold so high that only explicit sources make channels.
    let options = RiverOptions {
      min_catchment_km2: 1_000.0,
      ..RiverOptions::default()
    };
    let hydrology = build_hydrology(&map, &surface, &options, 5);
    let snouts = glacier_snouts(&hydrology);

    assert_eq!(snouts.len(), 1);
    let (_, sy) = hydrology.sample_xy(snouts[0]);
    assert_eq!(sy, 39);
    assert!(hydrology.channel[snouts[0] as usize]);
    assert!(hydrology.streams().iter().any(|s| s.cells[0] == snouts[0]));

    let off = build_hydrology(
      &map,
      &surface,
      &RiverOptions {
        snowmelt: 0.0,
        ..options
      },
      5,
    );
    assert!(!off.channel.iter().any(|c| *c));
  }
}
