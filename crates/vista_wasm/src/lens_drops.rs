//! Raindrops on the camera lens: a small simulation on the CPU, binned
//! into screen tiles for the present pass (`present_main` in
//! `shaders/atmosphere.wgsl`).
//!
//! Positions and sizes are in screen heights, so drops stay round on any
//! aspect ratio and look the same at every resolution. Behaviour follows
//! size, as it does on glass: small drops bead, sit still and evaporate;
//! large ones run down the screen, swallowing the beads they touch and
//! leaving a trail of tiny beads behind them.
//!
//! Every drop is binned into every tile its outline touches, so the
//! shader, which only looks at its own tile's drops, draws each drop whole.

/// The most drops on the lens at once.
pub const MAX_LENS_DROPS: usize = 512;

/// Rows of the tile grid. Columns follow the aspect ratio, so tiles are
/// roughly square.
pub const TILE_ROWS: u32 = 18;

/// The most columns of the tile grid, for very wide canvases.
pub const MAX_TILE_COLUMNS: u32 = 64;

/// The most drops one tile holds; the largest are kept, since tiny beads
/// under big drops do not show.
pub const TILE_CAPACITY: usize = 24;

/// Entries in the bin buffer: the grid size, one entry per tile, and the
/// drop indices.
pub const BIN_WORDS: usize = 2 + (MAX_TILE_COLUMNS * TILE_ROWS) as usize * (1 + TILE_CAPACITY);

/// Drops in the smaller part of the size range bead; larger drops run.
const BEAD_SHARE: f32 = 0.6;

/// Running drops are this much longer along their path than across it.
const RUNNING_STRETCH: f32 = 1.25;

/// Seconds for the live count to close a gap to its target.
const SPAWN_SECONDS: f32 = 0.25;

/// Seconds a new drop takes to spread to its full size.
const GROW_SECONDS: f32 = 0.2;

/// Developer settings: drops on the lens in full rain, and the smallest
/// and largest diameter as fractions of the canvas height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LensDropSettings {
  pub count: u32,
  pub min_size: f32,
  pub max_size: f32,
}

#[derive(Clone, Copy, Debug)]
struct Drop {
  x: f32,
  y: f32,
  /// Radius when full grown, before evaporation.
  radius: f32,
  age: f32,
  /// Seconds a bead lasts before it has evaporated.
  life: f32,
  running: bool,
  /// Phase of a running drop's sideways wander.
  phase: f32,
  /// Distance a running drop has run since it left its last trail bead.
  trail: f32,
  /// Sideways share of a running drop's direction, -1 to 1.
  drift: f32,
}

impl Drop {
  /// The radius now: spreading when new, shrinking as a bead evaporates
  /// (its area falls steadily).
  fn current_radius(&self) -> f32 {
    let spread = (self.age / GROW_SECONDS).min(1.0);

    if self.running {
      self.radius * spread
    } else {
      self.radius * spread * (1.0 - self.age / self.life).max(0.0).sqrt()
    }
  }
}

/// The drops on the lens.
#[derive(Clone, Debug)]
pub struct LensDrops {
  drops: Vec<Drop>,
  state: u64,
  debt: f32,
}

impl LensDrops {
  /// An empty lens whose drops follow `seed`.
  pub fn new(seed: u64) -> Self {
    Self {
      drops: Vec::new(),
      state: seed ^ 0x9e37_79b9_7f4a_7c15,
      debt: 0.0,
    }
  }

  /// Drops on the lens now.
  pub fn len(&self) -> usize {
    self.drops.len()
  }

  /// Whether the lens is dry.
  pub fn is_empty(&self) -> bool {
    self.drops.is_empty()
  }

  /// A uniform random number from 0 to 1 (xorshift).
  fn random(&mut self) -> f32 {
    self.state ^= self.state << 13;
    self.state ^= self.state >> 7;
    self.state ^= self.state << 17;
    (self.state >> 40) as f32 / (1u64 << 24) as f32
  }

  fn spawn(&mut self, x: f32, y: f32, diameter: f32, settings: &LensDropSettings) {
    let range = settings.max_size - settings.min_size;
    let life = 4.0 + 5.0 * self.random();
    let phase = self.random() * std::f32::consts::TAU;
    self.drops.push(Drop {
      x,
      y,
      radius: diameter * 0.5,
      age: 0.0,
      life,
      running: range > 0.0 && diameter - settings.min_size >= range * BEAD_SHARE,
      phase,
      trail: 0.0,
      drift: 0.0,
    });
  }

  /// Advance by `dt` seconds of rain at `intensity` (0 to 1) on a canvas
  /// `aspect` wide per unit of height.
  pub fn advance(&mut self, dt: f32, intensity: f32, aspect: f32, settings: &LensDropSettings) {
    let dt = dt.clamp(0.0, 0.25);
    let min = settings.min_size.min(settings.max_size);
    let max = settings.max_size;
    let range = (max - min).max(1e-6);
    let target = (settings.count as f32 * intensity.clamp(0.0, 1.0))
      .round()
      .min(MAX_LENS_DROPS as f32) as usize;
    let before = self.drops.len();

    // Beads age; running drops slide at a speed that rises with size, with
    // a slow sideways wander, and lay trail beads.
    let mut trail = Vec::new();

    for drop in &mut self.drops {
      drop.age += dt;

      if !drop.running {
        continue;
      }

      let size = ((drop.radius * 2.0 - min) / range).clamp(BEAD_SHARE, 1.0);
      let speed = 0.15 + 0.45 * (size - BEAD_SHARE) / (1.0 - BEAD_SHARE);
      drop.drift = (drop.phase + drop.age * 1.7).sin() * 0.25;
      drop.x += drop.drift * speed * dt;
      drop.y += speed * dt;
      drop.trail += speed * dt;

      let diameter = drop.radius * 2.0;

      if drop.trail >= diameter * 0.6 {
        drop.trail = 0.0;
        let bead = (diameter * 0.3).max(min);
        trail.push((drop.x, drop.y - drop.radius * 1.4 - bead * 0.5, bead));
      }
    }

    // A running drop swallows the beads it touches, growing by area.
    for i in 0..self.drops.len() {
      if !self.drops[i].running {
        continue;
      }

      for j in 0..self.drops.len() {
        let (a, b) = (self.drops[i], self.drops[j]);

        if b.running || b.age >= b.life {
          continue;
        }

        let reach = a.current_radius() + b.current_radius();

        if (a.x - b.x).powi(2) + (a.y - b.y).powi(2) < reach * reach {
          let grown = (a.radius * a.radius + b.radius * b.radius).sqrt();
          self.drops[i].radius = grown.min(max * 0.5);
          // Evaporated at once, and removed below.
          self.drops[j].age = b.life;
        }
      }
    }

    self.drops.retain(|drop| {
      let reach = drop.radius * RUNNING_STRETCH;
      let gone = if drop.running {
        drop.y - reach > 1.0 || drop.x < -reach || drop.x > aspect + reach
      } else {
        drop.age >= drop.life
      };
      !gone
    });

    let removed = before.saturating_sub(self.drops.len());

    // Trail beads count towards the total: the spawner makes room for them.
    for (x, y, diameter) in trail {
      if self.drops.len() < target {
        self.spawn(x, y, diameter, settings);
      }
    }

    // New drops replace those that went, and close any gap to the target
    // over a quarter of a second, so the count never jumps.
    if self.drops.len() < target {
      let gap = (target - self.drops.len()) as f32;
      self.debt += removed as f32 + gap * dt / SPAWN_SECONDS;
    } else {
      self.debt = 0.0;
    }

    while self.debt >= 1.0 && self.drops.len() < target {
      self.debt -= 1.0;
      let x = self.random() * aspect;
      let y = self.random();
      // Real drop populations are mostly small ones.
      let skew = self.random();
      self.spawn(x, y, min + range.min(max - min) * skew * skew, settings);
    }
  }

  /// The drops for the shader: centre x and y and radius in screen heights,
  /// and w: a bead's fade (0 to 1), or for a running drop 2 plus its
  /// sideways direction mapped to 0 to 1.
  pub fn packed(&self) -> Vec<[f32; 4]> {
    self
      .drops
      .iter()
      .map(|drop| {
        let fade = (drop.age / GROW_SECONDS).min(1.0);
        let w = if drop.running {
          2.0 + (drop.drift * 0.5 + 0.5) * 0.999
        } else {
          fade
        };
        [drop.x, drop.y, drop.current_radius(), w]
      })
      .collect()
  }
}

/// Columns of the tile grid for a canvas of this aspect ratio.
pub fn tile_columns(aspect: f32) -> u32 {
  ((TILE_ROWS as f32 * aspect).round() as u32).clamp(1, MAX_TILE_COLUMNS)
}

/// How far a drop's outline reaches from its centre, in screen heights.
fn reach(drop: &[f32; 4]) -> f32 {
  drop[2] * if drop[3] >= 2.0 { RUNNING_STRETCH } else { 1.0 }
}

/// Bin drops into the tile grid of a `width` x `height` pixel canvas. The
/// result is the grid's columns and rows, then per tile (row by row) its
/// first index into the list that follows, shifted up 8 bits, plus its
/// drop count, then the list of drop indices.
pub fn bin(drops: &[[f32; 4]], width: u32, height: u32) -> Vec<u32> {
  let aspect = width as f32 / height.max(1) as f32;
  let columns = tile_columns(aspect);
  let tiles = (columns * TILE_ROWS) as usize;
  let tile_width = aspect / columns as f32;
  let tile_height = 1.0 / TILE_ROWS as f32;
  // A pixel counts as inside a drop at its centre, so half a pixel more
  // covers every pixel the outline touches, with room for rounding.
  let margin = 1.0 / height.max(1) as f32;
  let mut counts = vec![0u32; tiles];
  let mut slots = vec![0u32; tiles * TILE_CAPACITY];

  for (index, drop) in drops.iter().enumerate().take(MAX_LENS_DROPS) {
    let r = reach(drop) + margin;
    let first_column = ((drop[0] - r) / tile_width).floor().max(0.0) as u32;
    let last_column = (((drop[0] + r) / tile_width).floor().max(0.0) as u32).min(columns - 1);
    let first_row = ((drop[1] - r) / tile_height).floor().max(0.0) as u32;
    let last_row = (((drop[1] + r) / tile_height).floor().max(0.0) as u32).min(TILE_ROWS - 1);

    for row in first_row..=last_row {
      for column in first_column..=last_column {
        // Skip tiles the circle only reaches past a corner.
        let nearest_x = drop[0].clamp(column as f32 * tile_width, (column + 1) as f32 * tile_width);
        let nearest_y = drop[1].clamp(row as f32 * tile_height, (row + 1) as f32 * tile_height);

        if (nearest_x - drop[0]).powi(2) + (nearest_y - drop[1]).powi(2) > r * r {
          continue;
        }

        let tile = (row * columns + column) as usize;
        let list = &mut slots[tile * TILE_CAPACITY..(tile + 1) * TILE_CAPACITY];

        if (counts[tile] as usize) < TILE_CAPACITY {
          list[counts[tile] as usize] = index as u32;
          counts[tile] += 1;
        } else {
          // A full tile keeps its largest drops: this one replaces the
          // smallest if it is larger.
          let smallest = (0..TILE_CAPACITY)
            .min_by(|a, b| drops[list[*a] as usize][2].total_cmp(&drops[list[*b] as usize][2]))
            .unwrap_or(0);

          if drops[list[smallest] as usize][2] < drop[2] {
            list[smallest] = index as u32;
          }
        }
      }
    }
  }

  let used: u32 = counts.iter().sum();
  let mut bins = Vec::with_capacity(2 + tiles + used as usize);
  bins.extend([columns, TILE_ROWS]);
  let mut offset = 0u32;

  for count in &counts {
    bins.push(offset << 8 | count);
    offset += count;
  }

  for (tile, count) in counts.iter().enumerate() {
    bins.extend_from_slice(&slots[tile * TILE_CAPACITY..tile * TILE_CAPACITY + *count as usize]);
  }

  bins
}
/// Whether a drop covers a point, both in screen heights. Mirrors the
/// shape test in `present_main`.
pub fn covers(drop: &[f32; 4], x: f32, y: f32) -> bool {
  let (dx, dy) = (x - drop[0], y - drop[1]);
  let (along, across) = if drop[3] >= 2.0 {
    // Running drops are longer along their path.
    let side = (drop[3] - 2.0) / 0.999 * 2.0 - 1.0;
    let down = (1.0 - side * side).max(0.0).sqrt();
    (
      (dx * side + dy * down) / RUNNING_STRETCH,
      dx * down - dy * side,
    )
  } else {
    (dx, dy)
  };

  along * along + across * across < drop[2] * drop[2]
}

#[cfg(test)]
mod tests {
  use super::*;

  const SETTINGS: LensDropSettings = LensDropSettings {
    count: 120,
    min_size: 0.008,
    max_size: 0.05,
  };

  fn run(lens: &mut LensDrops, seconds: f32, intensity: f32, settings: &LensDropSettings) {
    for _ in 0..(seconds * 60.0) as u32 {
      lens.advance(1.0 / 60.0, intensity, 16.0 / 9.0, settings);
    }
  }

  #[test]
  fn the_count_follows_the_setting_and_the_rain() {
    let mut lens = LensDrops::new(7);
    run(&mut lens, 20.0, 1.0, &SETTINGS);
    let full = lens.len() as f32;
    assert!((full - 120.0).abs() <= 6.0, "{full}");

    let mut samples = Vec::new();

    for _ in 0..60 {
      run(&mut lens, 1.0 / 6.0, 1.0, &SETTINGS);
      samples.push(lens.len());
    }

    assert!(
      samples.iter().all(|n| (*n as f32 - 120.0).abs() <= 6.0),
      "{samples:?}"
    );

    let mut half = LensDrops::new(8);
    run(&mut half, 20.0, 0.5, &SETTINGS);
    assert!((half.len() as f32 - 60.0).abs() <= 3.0, "{}", half.len());

    run(&mut lens, 10.0, 0.0, &SETTINGS);
    assert!(lens.is_empty(), "{}", lens.len());
  }

  #[test]
  fn sizes_stay_within_the_range_and_only_large_drops_run() {
    let mut lens = LensDrops::new(3);

    for _ in 0..1200 {
      lens.advance(1.0 / 60.0, 1.0, 1.6, &SETTINGS);

      for drop in &lens.drops {
        let diameter = drop.radius * 2.0;
        assert!(
          (SETTINGS.min_size - 1e-6..=SETTINGS.max_size + 1e-6).contains(&diameter),
          "{diameter}"
        );
        let share = (diameter - SETTINGS.min_size) / (SETTINGS.max_size - SETTINGS.min_size);
        assert!(drop.running || share < BEAD_SHARE, "{share}");
      }
    }

    assert!(lens.drops.iter().any(|d| d.running));
    assert!(lens.drops.iter().any(|d| !d.running));
  }

  #[test]
  fn beads_stay_put_and_running_drops_run_down() {
    let settings = LensDropSettings {
      count: 2,
      min_size: 0.01,
      max_size: 0.05,
    };
    let mut lens = LensDrops::new(1);
    lens.spawn(0.5, 0.2, 0.012, &settings);
    lens.spawn(1.2, 0.2, 0.048, &settings);
    run(&mut lens, 0.5, 1.0, &settings);

    let bead = lens.drops.iter().find(|d| !d.running).unwrap();
    let runner = lens.drops.iter().find(|d| d.running).unwrap();
    assert_eq!((bead.x, bead.y), (0.5, 0.2));
    assert!(runner.y > 0.4, "{}", runner.y);
  }

  #[test]
  fn running_drops_swallow_beads_and_grow_by_area() {
    let settings = LensDropSettings {
      count: 2,
      min_size: 0.01,
      max_size: 0.1,
    };
    let mut lens = LensDrops::new(2);
    lens.spawn(0.5, 0.2, 0.08, &settings);
    lens.spawn(0.5, 0.25, 0.03, &settings);
    lens.drops[0].age = 1.0;
    lens.drops[1].age = 1.0;
    lens.advance(1.0 / 60.0, 0.0, 1.0, &settings);

    assert_eq!(lens.drops.iter().filter(|d| !d.running).count(), 0);
    let grown = lens.drops[0].radius * 2.0;
    assert!(
      (grown - (0.08f32 * 0.08 + 0.03 * 0.03).sqrt()).abs() < 1e-5,
      "{grown}"
    );

    // Never beyond the largest size.
    lens.spawn(0.5, 0.3, 0.09, &settings);
    lens.drops[1].age = 1.0;
    lens.advance(1.0 / 60.0, 0.0, 1.0, &settings);
    assert!(lens.drops[0].radius * 2.0 <= 0.1 + 1e-6);
  }

  #[test]
  fn the_same_seed_gives_the_same_drops() {
    let mut a = LensDrops::new(11);
    let mut b = LensDrops::new(11);
    run(&mut a, 5.0, 0.8, &SETTINGS);
    run(&mut b, 5.0, 0.8, &SETTINGS);
    assert_eq!(a.packed(), b.packed());
  }

  /// A pseudo-random drop set, weighted towards tile edges and corners.
  fn random_drops(case: u32, aspect: f32) -> Vec<[f32; 4]> {
    let mut lens = LensDrops::new(case as u64 + 100);
    let count = 1 + (lens.random() * 120.0) as usize;
    let columns = tile_columns(aspect) as f32;

    (0..count)
      .map(|_| {
        let mut x = lens.random() * aspect;
        let mut y = lens.random();

        // Snap some drops onto tile edges and the screen's corners.
        match (lens.random() * 4.0) as u32 {
          0 => x = (x / aspect * columns).round() * aspect / columns,
          1 => y = (y * TILE_ROWS as f32).round() / TILE_ROWS as f32,
          2 => {
            x = if lens.random() < 0.5 { 0.0 } else { aspect };
            y = if lens.random() < 0.5 { 0.0 } else { 1.0 };
          }
          _ => {}
        }

        let diameter = 0.004 + lens.random() * lens.random() * 0.08;
        let w = if lens.random() < 0.4 {
          2.0 + lens.random() * 0.999
        } else {
          0.2 + 0.8 * lens.random()
        };
        [x, y, diameter * 0.5, w]
      })
      .collect()
  }

  /// Coverage the way the shader finds it: only the drops in the pixel's
  /// own tile.
  fn tiled_coverage(drops: &[[f32; 4]], bins: &[u32], width: u32, height: u32) -> Vec<bool> {
    let (columns, rows) = (bins[0], bins[1]);
    let aspect = width as f32 / height as f32;
    let list = 2 + (columns * rows) as usize;
    let mut mask = vec![false; (width * height) as usize];

    for py in 0..height {
      for px in 0..width {
        let x = (px as f32 + 0.5) / height as f32;
        let y = (py as f32 + 0.5) / height as f32;
        let column = ((x / aspect * columns as f32) as u32).min(columns - 1);
        let row = ((y * rows as f32) as u32).min(rows - 1);
        let entry = bins[2 + (row * columns + column) as usize];
        let (offset, count) = ((entry >> 8) as usize, (entry & 255) as usize);
        mask[(py * width + px) as usize] = bins[list + offset..list + offset + count]
          .iter()
          .any(|i| covers(&drops[*i as usize], x, y));
      }
    }

    mask
  }

  #[test]
  fn tiles_never_clip_a_drop() {
    for case in 0..1000 {
      let (width, height) = if case % 5 == 4 {
        (180, 390)
      } else {
        (640, 360)
      };
      let aspect = width as f32 / height as f32;
      let drops = random_drops(case, aspect);
      let bins = bin(&drops, width, height);
      let tiles = (bins[0] * bins[1]) as usize;
      assert!(
        bins[2..2 + tiles]
          .iter()
          .all(|e| (e & 255) < TILE_CAPACITY as u32),
        "case {case} overflowed"
      );

      let tiled = tiled_coverage(&drops, &bins, width, height);
      // Brute force: every drop tested at every pixel it could reach (a
      // drop never covers a pixel beyond its outline's reach).
      let mut brute = vec![false; tiled.len()];

      for drop in &drops {
        let r = reach(drop) * height as f32 + 2.0;
        let (cx, cy) = (drop[0] * height as f32, drop[1] * height as f32);
        let xs = ((cx - r).max(0.0) as u32)..((cx + r).ceil().max(0.0) as u32).min(width);
        let ys = ((cy - r).max(0.0) as u32)..((cy + r).ceil().max(0.0) as u32).min(height);

        for py in ys {
          for px in xs.clone() {
            let x = (px as f32 + 0.5) / height as f32;
            let y = (py as f32 + 0.5) / height as f32;
            brute[(py * width + px) as usize] |= covers(drop, x, y);
          }
        }
      }

      if let Some(pixel) = (0..tiled.len()).find(|i| tiled[*i] != brute[*i]) {
        panic!(
          "case {case} differs at {}, {}",
          pixel as u32 % width,
          pixel as u32 / width
        );
      }
    }
  }

  #[test]
  fn full_tiles_keep_the_largest_drops() {
    let mut drops: Vec<[f32; 4]> = (0..40)
      .map(|i| [0.05, 0.03, 0.001 + i as f32 * 0.0001, 1.0])
      .collect();
    drops.swap(0, 39);
    let bins = bin(&drops, 640, 360);
    let entry = bins[2];
    assert_eq!(entry & 255, TILE_CAPACITY as u32);
    let list = 2 + (bins[0] * bins[1]) as usize;
    let kept = &bins[list..list + TILE_CAPACITY];
    let smallest_kept = kept
      .iter()
      .map(|i| drops[*i as usize][2])
      .fold(1.0, f32::min);
    let largest_dropped = drops
      .iter()
      .enumerate()
      .filter(|(i, _)| !kept.contains(&(*i as u32)))
      .map(|(_, d)| d[2])
      .fold(0.0, f32::max);
    assert!(smallest_kept > largest_dropped);
  }

  #[test]
  fn binning_is_fast_enough() {
    let settings = LensDropSettings {
      count: 512,
      ..SETTINGS
    };
    let mut lens = LensDrops::new(5);
    run(&mut lens, 10.0, 1.0, &settings);
    let started = std::time::Instant::now();

    for _ in 0..100 {
      lens.advance(1.0 / 60.0, 1.0, 16.0 / 9.0, &settings);
      std::hint::black_box(bin(&lens.packed(), 1920, 1080));
    }

    // Generous for an unoptimised test build; release is far faster.
    let per_frame = started.elapsed().as_secs_f64() * 10.0;
    assert!(per_frame < 2.0, "{per_frame} ms per frame");
  }
}
