//! The world-space candidate lattice that every tree and grass tuft is
//! placed on, and the rules for thinning it with distance.
//!
//! Every potential tree is a point on a lattice with a 3 m pitch (grass:
//! 0.35 m), jittered by a hash of its integer coordinates. The same hash
//! gives the point a rank `r` in [0, 1): the point is a tree where `r < p`,
//! `p` being the local probability from the cover texture. The far set
//! (built on the CPU) holds the points with `r < p x FAR_KEEP`, and the
//! tiles streamed around the camera by `tree_generate.wgsl` hold the rest,
//! so each point is placed exactly once. `lattice.wgsl` mirrors the hash
//! line for line.

/// Tree lattice pitch in metres: about 1,100 candidates per hectare.
pub const TREE_PITCH: f32 = 3.0;
/// Candidates per hectare in the `p` formula, `target_density / 1100`.
pub const TREE_CANDIDATES_PER_HECTARE: f32 = 1_100.0;
/// Grass lattice pitch in metres.
pub const GRASS_PITCH: f32 = 0.35;
/// Edge of a streamed tree tile, in metres.
pub const TREE_TILE_METRES: f32 = 64.0;
/// Edge of a streamed grass tile, in metres.
pub const GRASS_TILE_METRES: f32 = 16.0;
/// Jittered points move up to this fraction of a cell either way.
pub const JITTER: f32 = 0.45;
/// Least fraction of each point's probability the static far set holds:
/// the large, well-spaced trees that read from far away.
pub const FAR_KEEP: f32 = 0.12;
/// Trees the far set holds before it gives way to streamed tiles. The old
/// maximum density put about this many on the default map, all drawn at
/// every distance, so up to it the far set is the whole forest.
pub const FAR_SET_TREES: f32 = 50_000.0;
/// Ranks (relative to `p`) over which a thinned tree shrinks to nothing,
/// so trees never pop.
pub const FADE_BAND: f32 = 0.05;
/// Most a thinned tree grows in height to keep the canopy closed; beyond
/// it the crown only widens.
pub const MAX_GROWTH: f32 = 1.8;
/// Most tiles generated per frame, nearest first, so moving never stalls.
pub const TILES_PER_FRAME: usize = 8;
/// Salt for the second hash of a tree point: species, size and colour.
pub const TREE_TRAITS_SALT: u32 = 0x2545_f491;
/// Salt for the hash that picks a tree's variant, age class and lean.
pub const TREE_SHAPE_SALT: u32 = 0x5bd1_e995;
/// Salt that separates the grass lattice from the trees'.
pub const GRASS_SALT: u32 = 0x9e37_79b9;
/// Salt that separates riparian scrub's lattice from the trees'.
pub const RIPARIAN_SALT: u32 = 0x68e3_1da5;

/// The 3D PCG hash of Jarzynski and Olano (2020). `lattice.wgsl` has the
/// same function; unsigned arithmetic wraps identically in both.
pub fn pcg3d(v: [u32; 3]) -> [u32; 3] {
  let mut v = v.map(|x| x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223));
  v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[2]));
  v[1] = v[1].wrapping_add(v[2].wrapping_mul(v[0]));
  v[2] = v[2].wrapping_add(v[0].wrapping_mul(v[1]));
  v = v.map(|x| x ^ (x >> 16));
  v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[2]));
  v[1] = v[1].wrapping_add(v[2].wrapping_mul(v[0]));
  v[2] = v[2].wrapping_add(v[0].wrapping_mul(v[1]));
  v
}

/// The hash of lattice point `(ix, iz)` for a seed.
pub fn point_hash(ix: i32, iz: i32, seed: u32) -> [u32; 3] {
  pcg3d([ix as u32, iz as u32, seed])
}

/// A hash word as a number in [0, 1), from its top 24 bits: exact in f32,
/// on the CPU and the GPU alike.
pub fn unit(word: u32) -> f32 {
  (word >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// The seed a 64-bit seed offset gives the lattice.
pub fn lattice_seed(seed_offset: u64) -> u32 {
  (seed_offset ^ (seed_offset >> 32)) as u32
}

/// Lattice cells per side of a clump cell: 12 m for trees.
pub const CLUMP_CELLS: i32 = 4;
/// Salt for the clump noise.
pub const CLUMP_SALT: u32 = 0x3c6e_f372;

/// The clump factor at lattice point `(ix, iz)`, 0 to 4.6 and 1 on
/// average:
/// young trees gather round their parents, so trees stand in clumps of
/// about [`CLUMP_CELLS`] lattice cells with thinner ground between. It is
/// the bilinear blend of random corner values to the fourth power (most
/// ground is thin, a few clumps dense). The blend is a whole number
/// below 2^24, exact in f32, times 47 / 2^23 in one correctly rounded
/// multiplication, so the CPU and the GPU agree bit for bit (`clump` in
/// `lattice.wgsl`).
pub fn clump(ix: i32, iz: i32, seed: u32) -> f32 {
  let n = CLUMP_CELLS;
  let (cx, fx) = (ix.div_euclid(n), ix.rem_euclid(n));
  let (cz, fz) = (iz.div_euclid(n), iz.rem_euclid(n));
  let corner = |x: i32, z: i32| {
    let value = (pcg3d([x as u32, z as u32, seed ^ CLUMP_SALT])[0] >> 28) as i32;
    value * value * value * value
  };
  let sum = corner(cx, cz) * (n - fx) * (n - fz)
    + corner(cx + 1, cz) * fx * (n - fz)
    + corner(cx, cz + 1) * (n - fx) * fz
    + corner(cx + 1, cz + 1) * fx * fz;
  sum as f32 * (47.0 / 8_388_608.0)
}

/// The jittered position of lattice point `(ix, iz)` with pitch `pitch`,
/// from its hash: `((i + 0.5 + jitter) x pitch)` on each axis.
pub fn jittered(ix: i32, iz: i32, hash: [u32; 3], pitch: f32) -> [f32; 2] {
  let jitter = |word: u32| (unit(word) - 0.5) * (2.0 * JITTER);
  [
    (ix as f32 + 0.5 + jitter(hash[0])) * pitch,
    (iz as f32 + 0.5 + jitter(hash[1])) * pitch,
  ]
}

/// Tree density at slider value `d` (0 to 4), in trees per hectare where
/// the land fully supports them. 1 is the old maximum, 4 a closed canopy.
pub fn target_density(d: f32) -> f32 {
  const TABLE: [(f32, f32); 6] = [
    (0.0, 0.0),
    (0.35, 25.0),
    (1.0, 70.0),
    (2.0, 220.0),
    (3.0, 480.0),
    (4.0, 900.0),
  ];
  interpolate(&TABLE, d)
}

/// Interpolate `table`, whose first column rises, at `x`, clamped to its
/// ends.
fn interpolate(table: &[(f32, f32)], x: f32) -> f32 {
  let x = x.clamp(table[0].0, table[table.len() - 1].0);
  let upper = table
    .iter()
    .position(|(at, _)| *at >= x)
    .unwrap_or(table.len() - 1)
    .max(1);
  let ((x0, y0), (x1, y1)) = (table[upper - 1], table[upper]);
  y0 + (y1 - y0) * (x - x0) / (x1 - x0)
}

/// The share of grass lattice points that grow a tuft on ideal meadow at
/// slider value `d` (0 to 4): 0.5, the default, is a natural meadow
/// whose tufts cover at least 70 % of the ground near the camera (see
/// `grass::TUFT_COVER_RADIUS`), 1 a lush meadow, and 4 every point.
pub fn grass_probability(d: f32) -> f32 {
  interpolate(&[(0.0, 0.0), (0.5, 0.45), (1.0, 0.65), (4.0, 1.0)], d)
}

/// How tall meadow grass grows at slider value `d`, relative to the
/// default: taller in a lush meadow, and long at the maximum.
pub fn grass_height(d: f32) -> f32 {
  interpolate(&[(0.0, 1.0), (0.5, 1.0), (1.0, 1.15), (4.0, 1.4)], d)
}

/// The share of each point's probability the far set holds, for a forest
/// of `trees` in all: every tree up to [`FAR_SET_TREES`], then less, but
/// never under [`FAR_KEEP`]. At 1 there are no streamed tiles, no
/// thinning and no canopy layer, and every tree is drawn as placed.
pub fn far_keep(trees: f32) -> f32 {
  if trees <= FAR_SET_TREES {
    1.0
  } else {
    (FAR_SET_TREES / trees).max(FAR_KEEP)
  }
}

/// A tree crown's area in square metres, for canopy cover: a mature
/// crown about 8 m across.
pub const CROWN_AREA: f32 = 50.0;

/// Crown area per square metre of ground, per unit of cover share, at
/// tree slider value `d`: `target_density x CROWN_AREA / 10,000`.
pub fn canopy_density(d: f32) -> f32 {
  target_density(d) * (CROWN_AREA / 10_000.0)
}

/// The share of the ground under canopy where the cover texture's red
/// channel is `red`: `1 - exp(-canopy x share)`, crowns overlapping at
/// random, with `canopy` from [`canopy_density`].
pub fn canopy_cover(red: f32, canopy: f32) -> f32 {
  let share = red * red * (crate::render::flora::COVER_SHARE_MAX / 65_025.0);
  1.0 - (-canopy * share).exp()
}

/// How one candidate is thinned at a distance from the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thinning {
  /// Candidates with a rank (relative to `p`) below this are kept.
  pub keep: f32,
  /// Ranks from `keep` to `keep + band` shrink to nothing.
  pub band: f32,
  /// Height scale of a kept tree.
  pub height: f32,
  /// Extra crown width beyond `height`, where it is capped.
  pub width: f32,
  /// The share of the candidates' crown area kept before growth: what
  /// the thinned trees would cover without growing.
  pub share: f32,
}

/// Thinning at `distance` from the camera, with full density within
/// `radius` and never fewer than `floor` of the candidates kept:
/// `keep = max(floor, min(1, (radius / distance)^2))`. Kept trees grow by
/// `sqrt(1 / kept share)`, so their crown area keeps the canopy cover
/// constant: in height up to [`MAX_GROWTH`], and in width beyond it.
pub fn thinning(distance: f32, radius: f32, floor: f32) -> Thinning {
  let ratio = radius / distance.max(1e-3);
  let keep = (ratio * ratio).clamp(floor, 1.0);
  let band = FADE_BAND.min(keep - floor);
  // The kept share of crown area: all below `keep`, and the fading band
  // weighted by its squared size, truncated at rank 1.
  let end = (keep + band).min(1.0);
  let share = if band > 0.0 {
    let cut = (keep + band - end) / band;
    keep + band / 3.0 * (1.0 - cut * cut * cut)
  } else {
    keep
  };
  let growth = (1.0 / share.max(1e-3)).sqrt();
  let height = growth.min(MAX_GROWTH);
  Thinning {
    keep,
    band,
    height,
    width: growth / height,
    share,
  }
}

impl Thinning {
  /// The size factor of a candidate of rank `u` (relative to `p`): 1 when
  /// kept, 0 when dropped, and shrinking across the fading band.
  pub fn fade(&self, u: f32) -> f32 {
    if u < self.keep {
      1.0
    } else if self.band > 0.0 {
      ((self.keep + self.band - u) / self.band).clamp(0.0, 1.0)
    } else {
      0.0
    }
  }
}

/// A distance class of tile slots: tiles whose nearest point lies within
/// `reach` of the camera, and beyond the previous class's reach, go in
/// one of `slots` slots of `capacity` instances each.
#[derive(Clone, Debug, PartialEq)]
pub struct TileClass {
  /// Outer distance, in metres, of a tile's nearest point.
  pub reach: f32,
  /// Instances each slot holds.
  pub capacity: u32,
  /// Slots in the class.
  pub slots: u32,
  /// The class's first slot.
  pub first_slot: u32,
  /// The class's first instance in the pool.
  pub first_instance: u32,
}

/// How a pool of streamed instances is divided into tile slots.
#[derive(Clone, Debug, PartialEq)]
pub struct TileLayout {
  /// Tile edge in metres.
  pub tile: f32,
  /// Classes from the nearest out.
  pub classes: Vec<TileClass>,
}

/// An upper bound, rarely exceeded, on a Poisson count of mean `mean`.
pub fn count_bound(mean: f32) -> u32 {
  (mean + 3.5 * mean.sqrt() + 2.0).ceil() as u32
}

/// The distance from `(x, z)` to the square tile `(tx, tz)` of edge
/// `tile`: 0 inside it.
pub fn tile_distance(tile: f32, tx: i32, tz: i32, x: f32, z: f32) -> f32 {
  let gap = |p: f32, t: i32| {
    let low = t as f32 * tile;
    (low - p).max(p - low - tile).max(0.0)
  };
  crate::maths::length2(gap(x, tx), gap(z, tz))
}

impl TileLayout {
  /// Classes out to each of `reaches`, each slot holding `capacity(d)`
  /// instances for a tile whose nearest point is `d` from the camera (the
  /// class's inner reach). Each class has as many slots as tiles can fall
  /// in it wherever the camera stands, with a little slack.
  // `dyn` rather than generic: one copy in the binary serves both kinds.
  pub fn new(tile: f32, reaches: &[f32], capacity: &dyn Fn(f32) -> u32) -> Self {
    let outer = reaches.last().copied().unwrap_or(0.0);
    let span = (outer / tile).ceil() as i32 + 1;
    let mut most = vec![0u32; reaches.len()];

    // The camera stands anywhere in a tile; these positions bound the
    // counts closely enough.
    for step in 0..64 {
      let (x, z) = (
        (step % 8) as f32 / 7.0 * tile,
        (step / 8) as f32 / 7.0 * tile,
      );
      let mut counts = vec![0u32; reaches.len()];

      for tz in -span..=span {
        for tx in -span..=span {
          let distance = tile_distance(tile, tx, tz, x, z);

          if let Some(class) = reaches.iter().position(|reach| distance <= *reach) {
            counts[class] += 1;
          }
        }
      }

      for (most, count) in most.iter_mut().zip(counts) {
        *most = (*most).max(count);
      }
    }

    let mut classes = Vec::with_capacity(reaches.len());
    let (mut first_slot, mut first_instance, mut inner) = (0, 0, 0.0);

    for (reach, count) in reaches.iter().zip(most) {
      let slots = count + count / 20 + 2;
      let capacity = capacity(inner).max(1);
      classes.push(TileClass {
        reach: *reach,
        capacity,
        slots,
        first_slot,
        first_instance,
      });
      first_slot += slots;
      first_instance += slots * capacity;
      inner = *reach;
    }

    Self { tile, classes }
  }

  /// Slots in every class.
  pub fn slot_count(&self) -> u32 {
    self.classes.iter().map(|class| class.slots).sum()
  }

  /// Instances in every slot of every class.
  pub fn instance_count(&self) -> u32 {
    self
      .classes
      .iter()
      .map(|class| class.slots * class.capacity)
      .sum()
  }

  /// The class a tile `distance` from the camera belongs to.
  pub fn class_at(&self, distance: f32) -> Option<usize> {
    self
      .classes
      .iter()
      .position(|class| distance <= class.reach)
  }

  /// The class holding slot `slot`.
  fn class_of(&self, slot: u32) -> usize {
    self
      .classes
      .iter()
      .rposition(|class| class.first_slot <= slot)
      .unwrap_or(0)
  }

  /// The first instance and capacity of slot `slot`.
  pub fn slot_range(&self, slot: u32) -> (u32, u32) {
    let class = &self.classes[self.class_of(slot)];
    (
      class.first_instance + (slot - class.first_slot) * class.capacity,
      class.capacity,
    )
  }
}

/// A tile to generate this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileJob {
  /// Slot it goes in.
  pub slot: u32,
  /// Tile coordinates.
  pub tile: [i32; 2],
  /// Candidates are stored below this rank (relative to `p`).
  pub keep: f32,
}

/// A tile in a slot, and the rank below which it was generated.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Resident {
  tile: [i32; 2],
  keep: f32,
}

/// Which tile sits in each slot of a [`TileLayout`], kept up to date as
/// the camera moves.
#[derive(Clone, Debug, Default)]
pub struct TileRing {
  slots: Vec<Option<Resident>>,
}

/// What [`TileRing::update`] changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TileChanges {
  /// Tiles to generate, nearest first.
  pub jobs: Vec<TileJob>,
  /// Slots emptied: their instance counts go to 0.
  pub freed: Vec<u32>,
}

impl TileRing {
  /// An empty ring for `layout`.
  pub fn new(layout: &TileLayout) -> Self {
    Self {
      slots: vec![None; layout.slot_count() as usize],
    }
  }

  /// Every slot's tile, for tests and the budget estimate.
  pub fn live(&self) -> impl Iterator<Item = (u32, [i32; 2])> + '_ {
    self
      .slots
      .iter()
      .enumerate()
      .filter_map(|(slot, resident)| resident.map(|resident| (slot as u32, resident.tile)))
  }

  /// Bring the ring up to date for a camera at `(x, z)`: every tile whose
  /// nearest point is within the layout's reach and that `wanted` accepts
  /// belongs in a slot of its distance class, generated below
  /// `keep_at(distance)`. At most `max_jobs` tiles are (re)generated,
  /// nearest first; tiles no longer wanted free their slots.
  pub fn update(
    &mut self,
    layout: &TileLayout,
    camera: [f32; 2],
    wanted: &dyn Fn(i32, i32) -> bool,
    keep_at: &dyn Fn(f32) -> f32,
    max_jobs: usize,
  ) -> TileChanges {
    let tile = layout.tile;
    let outer = layout.classes.last().map_or(0.0, |class| class.reach);
    let range = |p: f32| {
      (
        ((p - outer) / tile).floor() as i32,
        ((p + outer) / tile).floor() as i32,
      )
    };
    let ((x0, x1), (z0, z1)) = (range(camera[0]), range(camera[1]));
    // Every tile that can be wanted lies in this window round the camera,
    // so plain vectors over it index tiles; nothing needs hashing.
    let columns = x1 - x0 + 1;
    let cell = |t: [i32; 2]| {
      (t[0] >= x0 && t[0] <= x1 && t[1] >= z0 && t[1] <= z1)
        .then(|| ((t[1] - z0) * columns + t[0] - x0) as usize)
    };
    let cells = (columns * (z1 - z0 + 1)) as usize;
    let mut resident: Vec<Option<u32>> = vec![None; cells];
    let mut desired: Vec<Option<(usize, f32)>> = vec![None; cells];
    let mut pending = Vec::new();

    for (slot, held) in self.slots.iter().enumerate() {
      if let Some(index) = held.and_then(|held| cell(held.tile)) {
        resident[index] = Some(slot as u32);
      }
    }

    for tz in z0..=z1 {
      for tx in x0..=x1 {
        let distance = tile_distance(tile, tx, tz, camera[0], camera[1]);
        let Some(class) = layout.class_at(distance) else {
          continue;
        };

        if !wanted(tx, tz) {
          continue;
        }

        let keep = keep_at(distance);
        let index = ((tz - z0) * columns + tx - x0) as usize;
        desired[index] = Some((class, keep));
        let current = resident[index].and_then(|slot| {
          let held = self.slots[slot as usize]?;
          (layout.class_of(slot) == class && held.keep >= keep - 1e-4).then_some(())
        });

        if current.is_none() {
          pending.push((distance, [tx, tz], class, keep));
        }
      }
    }

    let mut changes = TileChanges::default();
    let wanted_class = |t: [i32; 2]| {
      cell(t)
        .and_then(|index| desired[index])
        .map(|(class, _)| class)
    };

    // Tiles no longer wanted anywhere leave their slots now.
    for slot in 0..self.slots.len() {
      if let Some(held) = self.slots[slot] {
        if wanted_class(held.tile).is_none() {
          self.slots[slot] = None;
          changes.freed.push(slot as u32);
        }
      }
    }

    for _ in 0..max_jobs {
      // The nearest pending tile: a few jobs a frame, so picking each
      // beats sorting them all.
      let Some(next) = (0..pending.len()).min_by(|a, b| pending[*a].0.total_cmp(&pending[*b].0))
      else {
        break;
      };
      let (_, coords, class, keep) = pending.swap_remove(next);
      let spec = &layout.classes[class];
      let slots = spec.first_slot..spec.first_slot + spec.slots;
      // A free slot, or else one holding a tile that is due to move to
      // another class or be regenerated: the farthest such.
      let free = slots
        .clone()
        .find(|slot| self.slots[*slot as usize].is_none());
      let slot = free.or_else(|| {
        slots
          .filter(|slot| {
            self.slots[*slot as usize]
              .is_some_and(|held| wanted_class(held.tile).is_some_and(|wanted| wanted != class))
          })
          .max_by(|a, b| {
            let distance = |slot: &u32| {
              let t = self.slots[*slot as usize].map_or([0, 0], |held| held.tile);
              tile_distance(tile, t[0], t[1], camera[0], camera[1])
            };
            distance(a).total_cmp(&distance(b))
          })
      });
      let Some(slot) = slot else {
        continue;
      };

      if let Some(old) = self.slots[slot as usize].and_then(|old| cell(old.tile)) {
        resident[old] = None;
      }

      let index = cell(coords).unwrap_or(0);

      if let Some(previous) = resident[index] {
        if previous != slot {
          self.slots[previous as usize] = None;
          changes.freed.push(previous);
        }
      }

      resident[index] = Some(slot);
      self.slots[slot as usize] = Some(Resident { tile: coords, keep });
      changes.jobs.push(TileJob {
        slot,
        tile: coords,
        keep,
      });
    }

    changes
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// `pcg3d` as `lattice.wgsl` writes it, vector operation by vector
  /// operation: `v = v * 1664525u + 1013904223u`, then the three cross
  /// additions, `v ^= v >> 16u`, and the cross additions again.
  fn pcg3d_as_wgsl(v: [u32; 3]) -> [u32; 3] {
    let mul = |a: [u32; 3], s: u32| a.map(|x| x.wrapping_mul(s));
    let add = |a: [u32; 3], s: u32| a.map(|x| x.wrapping_add(s));
    let mut v = add(mul(v, 1_664_525), 1_013_904_223);
    let cross = |v: &mut [u32; 3]| {
      v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[2]));
      v[1] = v[1].wrapping_add(v[2].wrapping_mul(v[0]));
      v[2] = v[2].wrapping_add(v[0].wrapping_mul(v[1]));
    };
    cross(&mut v);
    v = [
      v[0] ^ (v[0] >> 16),
      v[1] ^ (v[1] >> 16),
      v[2] ^ (v[2] >> 16),
    ];
    cross(&mut v);
    v
  }

  #[test]
  fn the_hash_matches_the_shader_on_ten_thousand_points() {
    let wgsl = include_str!("../shaders/lattice.wgsl");
    // The shader still holds the arithmetic this port follows.
    for line in [
      "v = v * 1664525u + 1013904223u;",
      "v.x += v.y * v.z;",
      "v.y += v.z * v.x;",
      "v.z += v.x * v.y;",
      "v ^= v >> vec3<u32>(16u);",
      "return f32(word >> 8u) * (1.0 / 16777216.0);",
    ] {
      assert!(wgsl.contains(line), "lattice.wgsl no longer has {line}");
    }

    for i in 0..10_000i32 {
      let (ix, iz) = (i % 100 - 50, i / 100 * 37 - 1800);
      let seed = (i as u32).wrapping_mul(2_654_435_761);
      let v = [ix as u32, iz as u32, seed];
      assert_eq!(pcg3d(v), pcg3d_as_wgsl(v));
    }

    // Reference values, so neither side can drift.
    assert_eq!(pcg3d([0, 0, 0]), pcg3d_as_wgsl([0, 0, 0]));
    assert_ne!(pcg3d([1, 0, 0]), pcg3d([0, 1, 0]));
    assert!(unit(u32::MAX) < 1.0 && unit(0) == 0.0);
  }

  #[test]
  fn clumps_average_one() {
    let mut total = 0.0f64;

    for iz in -200..200 {
      for ix in -200..200 {
        let c = clump(ix, iz, 5);
        assert!((0.0..4.6).contains(&c));
        total += f64::from(c);
      }
    }

    assert!(
      (total / 160_000.0 - 1.0).abs() < 0.08,
      "{}",
      total / 160_000.0
    );
    // Neighbours are alike; clumps are 12 m, not per tree.
    assert!((clump(0, 0, 5) - clump(1, 0, 5)).abs() <= 4.6 / 4.0);
  }

  #[test]
  fn ranks_are_uniform() {
    let mut buckets = [0u32; 10];

    for iz in 0..100 {
      for ix in 0..100 {
        buckets[(unit(point_hash(ix, iz, 7)[2]) * 10.0) as usize] += 1;
      }
    }

    assert!(
      buckets.iter().all(|count| (900..1100).contains(count)),
      "{buckets:?}"
    );
  }

  #[test]
  fn the_density_table_is_piecewise_linear() {
    assert_eq!(target_density(0.0), 0.0);
    assert_eq!(target_density(0.35), 25.0);
    assert_eq!(target_density(1.0), 70.0);
    assert_eq!(target_density(2.0), 220.0);
    assert_eq!(target_density(3.0), 480.0);
    assert_eq!(target_density(4.0), 900.0);
    assert!((target_density(1.5) - 145.0).abs() < 1e-3);
    assert!((target_density(0.175) - 12.5).abs() < 1e-3);
    assert_eq!(target_density(9.0), 900.0);
    // A closed canopy stays below one tree per lattice point.
    assert!(target_density(4.0) / TREE_CANDIDATES_PER_HECTARE < 1.0);
  }

  #[test]
  fn grass_rises_from_a_natural_meadow_to_full_cover() {
    assert_eq!(grass_probability(0.0), 0.0);
    assert!((grass_probability(0.5) - 0.45).abs() < 1e-6);
    assert!((grass_probability(1.0) - 0.65).abs() < 1e-6);
    assert!((grass_probability(4.0) - 1.0).abs() < 1e-6);
    assert!(grass_probability(2.0) > grass_probability(1.0));
    assert!(grass_probability(3.0) < grass_probability(4.0));
    assert!((grass_probability(0.25) - 0.225).abs() < 1e-6);
    // Lusher meadows grow taller, never shorter.
    assert_eq!(grass_height(0.5), 1.0);
    assert!(grass_height(1.0) > 1.0 && grass_height(4.0) > grass_height(1.0));
  }

  #[test]
  fn the_far_set_holds_small_forests_whole() {
    assert_eq!(far_keep(0.0), 1.0);
    assert_eq!(far_keep(FAR_SET_TREES), 1.0);
    assert_eq!(far_keep(2.0 * FAR_SET_TREES), 0.5);
    assert_eq!(far_keep(100.0 * FAR_SET_TREES), FAR_KEEP);
    // Nothing thins while the far set is whole.
    let t = thinning(5_000.0, 250.0, far_keep(1_000.0));
    assert_eq!((t.keep, t.height, t.width), (1.0, 1.0, 1.0));
    assert_eq!(t.fade(2046.0 / 2047.0), 1.0);
  }

  #[test]
  fn thinning_keeps_crown_area_and_caps_height() {
    let near = thinning(100.0, 250.0, FAR_KEEP);
    assert_eq!(
      (near.keep, near.height, near.width, near.share),
      (1.0, 1.0, 1.0, 1.0)
    );
    assert_eq!(near.fade(0.999), 1.0);

    for distance in [300.0, 500.0, 800.0, 1600.0] {
      let t = thinning(distance, 250.0, FAR_KEEP);
      assert!(t.keep >= FAR_KEEP && t.keep < 1.0);
      assert!(t.height <= MAX_GROWTH + 1e-6 && t.width >= 1.0);
      // Integrate the kept crown area over ranks: it is the full area.
      let steps = 100_000;
      let area: f32 = (0..steps)
        .map(|i| {
          let u = (i as f32 + 0.5) / steps as f32;
          let size = t.fade(u) * t.height * t.width;
          size * size
        })
        .sum::<f32>()
        / steps as f32;
      assert!((area - 1.0).abs() < 0.01, "{distance} m: {area}");
    }

    // Beyond the tiles only the far set is left, unfaded.
    let far = thinning(5_000.0, 250.0, FAR_KEEP);
    assert_eq!((far.keep, far.band), (FAR_KEEP, 0.0));
    assert_eq!(far.fade(FAR_KEEP + 0.01), 0.0);
  }

  #[test]
  fn tiles_stream_nearest_first_and_leave_behind() {
    let layout = TileLayout::new(64.0, &[200.0, 400.0], &|d| if d < 1.0 { 100 } else { 10 });
    assert_eq!(layout.classes[0].capacity, 100);
    assert_eq!(layout.classes[1].capacity, 10);
    assert_eq!(
      layout.classes[1].first_instance,
      layout.classes[0].slots * 100
    );
    let mut ring = TileRing::new(&layout);
    let wanted = |_: i32, _: i32| true;
    let mut generated = 0;
    let mut last = 0.0;

    // The first frames fill the ring, eight tiles a frame, nearest first.
    loop {
      let changes = ring.update(&layout, [10.0, 10.0], &wanted, &|_| 1.0, TILES_PER_FRAME);

      if changes.jobs.is_empty() {
        break;
      }

      assert!(changes.jobs.len() <= TILES_PER_FRAME);

      for job in &changes.jobs {
        let distance = tile_distance(64.0, job.tile[0], job.tile[1], 10.0, 10.0);
        assert!(distance >= last - 1e-3);
        last = distance;
        let (first, capacity) = layout.slot_range(job.slot);
        assert!(first + capacity <= layout.instance_count());
      }

      generated += changes.jobs.len();
    }

    assert_eq!(generated, ring.live().count());
    let tiles: Vec<[i32; 2]> = ring.live().map(|(_, tile)| tile).collect();
    assert!(tiles.contains(&[0, 0]) && tiles.contains(&[-6, 0]));

    // Moving on frees the tiles left behind and fills the new ones.
    let mut freed = 0;

    for _ in 0..40 {
      let changes = ring.update(&layout, [700.0, 10.0], &wanted, &|_| 1.0, TILES_PER_FRAME);
      freed += changes.freed.len();
    }

    assert!(freed > 0);
    assert!(ring
      .live()
      .all(|(_, tile)| tile_distance(64.0, tile[0], tile[1], 700.0, 10.0) <= 400.0));

    // A tile generated below a rank is generated again when it needs
    // more.
    let changes = ring.update(&layout, [700.0, 10.0], &wanted, &|_| 1.0, 1000);
    assert!(changes.jobs.is_empty());
    let changes = ring.update(&layout, [700.0, 10.0], &wanted, &|_| 1.5, 1000);
    assert_eq!(changes.jobs.len(), ring.live().count());
  }

  #[test]
  fn unwanted_tiles_are_never_generated() {
    let layout = TileLayout::new(64.0, &[300.0], &|_| 10);
    let mut ring = TileRing::new(&layout);
    let changes = ring.update(&layout, [0.0, 0.0], &|x, _| x >= 0, &|_| 1.0, 1000);
    assert!(!changes.jobs.is_empty());
    assert!(changes.jobs.iter().all(|job| job.tile[0] >= 0));
  }
}
