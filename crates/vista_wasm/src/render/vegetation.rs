//! Streamed vegetation: the per-terrain data the tree and grass generators
//! read, the CPU mirror of `tree_generate.wgsl`, the static far set built
//! from it, and the channel bins every generator keeps out of the water.
//!
//! Trees are points of the lattice in `render/lattice.rs`. The far set,
//! built here once per terrain, holds the points with `r < p x far_keep`
//! (all of them while the forest is small, see `lattice::far_keep`);
//! the GPU fills 64 m tiles around the camera with the rest. Both decide
//! each point by [`tree_at`] (or its WGSL twin), so they agree point for
//! point.

use vista_types::BiomeKind;

use crate::render::flora::{
  species_niche, TreeInstance, Water, TREE_AGE_SHIFT, TREE_GROUNDED, TREE_LATTICE, TREE_LEAN_SHIFT,
  TREE_RANK_SHIFT, TREE_STUNTED, TREE_VARIANT_SHIFT,
};
use crate::render::lattice::{
  jittered, point_hash, unit, RIPARIAN_SALT, TREE_PITCH, TREE_SHAPE_SALT, TREE_TRAITS_SALT,
};
use crate::render::tree_growth::Age;
use crate::render::tree_models::{TreeSpecies, PREVAILING_WIND};
use crate::terrain::biomes::{celsius_to_unit, SurfaceSample};
use crate::terrain::heightmap::HeightMap;

/// Most texels per side of the per-terrain textures: larger maps are read
/// every few samples.
pub const TEXTURE_MAX: u32 = 2048;

/// Heightmap samples per texel of the per-terrain textures.
pub fn texture_stride(width: u32, height: u32) -> u32 {
  (width.max(height).saturating_sub(1) / (TEXTURE_MAX - 1)).max(1)
}

/// Trees stand at least this high above sea level, in metres.
pub const TREE_WATER_LINE: f32 = 0.6;
/// Trees keep this far from lakes and rivers, in metres.
pub const TREE_WATER_CLEARANCE: f32 = 1.0;
/// Trunks keep this far from a boulder's footprint, in metres.
pub const TRUNK_CLEARANCE: f32 = 0.5;
/// Half the span, in metres, over which a candidate's own slope is
/// measured.
pub const SLOPE_SPAN: f32 = 1.5;
/// Share of lattice trees that are young: the seedlings that gather
/// around their parents, 0.45 to 0.8 of full size. Out of 256.
pub const YOUNG_SHARE: u32 = 90;
/// Largest rank stored in a tree's species word ([`TREE_RANK_SHIFT`]).
pub const RANK_LEVELS: u32 = 2047;
/// Trees within this many metres of a channel's centreline lean over the
/// water, reaching for the light above it.
pub const LEAN_WATER_METRES: f32 = 8.0;
/// Share of mature lattice trees that are old, out of 256.
pub const OLD_SHARE: u32 = 70;

/// The surface texel for one sample: r temperature unit ((°C + 30) / 65),
/// g moisture, b permanent snow (fast ice on the sea), a biome index.
pub fn surface_texel(sample: &SurfaceSample) -> [u8; 4] {
  let temperature = celsius_to_unit(sample.celsius()).clamp(0.0, 1.0);

  [
    (temperature * 255.0).round() as u8,
    sample.moisture,
    sample.permanent_snow,
    sample.biome,
  ]
}

/// The species a cover texture names where it has trees, one bit each.
pub fn cover_species(cover: &[[u8; 4]]) -> u32 {
  cover
    .iter()
    .filter(|texel| texel[0] > 0)
    .fold(0, |mask, texel| {
      mask | 1 << texel[1].min(7) | 1 << texel[2].min(7)
    })
}

/// Everything the generators read about the ground, at the resolution of
/// the per-terrain textures: the same numbers the GPU textures hold.
#[derive(Clone, Debug, Default)]
pub struct GroundData {
  /// Texels per row.
  pub width: u32,
  /// Rows of texels.
  pub height: u32,
  /// Heightmap samples per texel.
  pub stride: u32,
  /// Terrain half extents in metres.
  pub half: [f32; 2],
  /// Metres per texel.
  pub texel_metres: f32,
  /// Sea level in metres.
  pub sea: f32,
  /// Heights (the height texture).
  pub heights: Vec<f32>,
  /// The surface texture (see [`surface_texel`]).
  pub surface: Vec<[u8; 4]>,
  /// `surface_texture_b`: distance to water / 40 m, snow and ice cover,
  /// bankside greening, and the talus field where boulders may lie
  /// (`SurfaceSample::talus_here`).
  pub banks: Vec<[u8; 4]>,
  /// The cover texture (see `flora::bake_cover`).
  pub cover: Vec<[u8; 4]>,
  /// The grass texture (see `grass::bake_grass`).
  pub grass: Vec<[u8; 4]>,
  /// The grass density mask, one byte a texel capped to density 4 (see
  /// `painted::density_multiplier`), or empty without one.
  pub grass_mask: Vec<u8>,
}

impl GroundData {
  /// The mapping and the heights of `map`; the other textures are empty
  /// until set.
  pub fn of(map: &HeightMap) -> Self {
    let (width, height) = (map.metadata.width, map.metadata.height);

    if width == 0 || height == 0 {
      return Self::default();
    }

    let stride = texture_stride(width, height);
    let (columns, rows) = ((width - 1) / stride + 1, (height - 1) / stride + 1);
    let metres = map.metadata.metres_per_sample.max(0.001);
    let sea = map.metadata.sea_level_metres;
    let heights = (0..columns * rows)
      .map(|texel| {
        let index = ((texel / columns) * stride * width + (texel % columns) * stride) as usize;

        if map.no_data[index] {
          sea - 50.0
        } else {
          map.heights[index]
        }
      })
      .collect();

    Self {
      width: columns,
      height: rows,
      stride,
      half: [
        (width as f32 - 1.0) * metres * 0.5,
        (height as f32 - 1.0) * metres * 0.5,
      ],
      texel_metres: metres * stride as f32,
      sea,
      heights,
      ..Self::default()
    }
  }

  /// The heightmap sample index under texel `texel`.
  pub fn sample_of(&self, map: &HeightMap, texel: usize) -> usize {
    let (x, z) = (texel as u32 % self.width, texel as u32 / self.width);
    ((z * self.stride).min(map.metadata.height - 1) * map.metadata.width
      + (x * self.stride).min(map.metadata.width - 1)) as usize
  }

  /// Fill the surface textures from the classified surface (with its
  /// talus field), the wet-bank field (distance to water) and the bankside
  /// greening (`riparian`, per sample, or empty).
  pub fn set_surface(
    &mut self,
    map: &HeightMap,
    surface: &[SurfaceSample],
    wet: &crate::render::water::WetBanks,
    riparian: &[u8],
  ) {
    let texels = (self.width * self.height) as usize;

    if surface.len() != map.heights.len() {
      self.surface = vec![[0; 4]; texels];
      self.banks = vec![[255, 0, 0, 0]; texels];
      return;
    }

    // Without water the distance is the field's full range everywhere.
    let dry = wet.distance.len() != texels;
    self.surface = Vec::with_capacity(texels);
    self.banks = Vec::with_capacity(texels);

    for texel in 0..texels {
      let index = self.sample_of(map, texel);
      let sample = &surface[index];
      self.surface.push(surface_texel(sample));
      self.banks.push([
        if dry { 255 } else { wet.distance[texel] },
        sample.snow_cover(),
        riparian.get(index).copied().unwrap_or(0),
        sample.talus_here(),
      ]);
    }
  }

  /// Texel coordinates (fractional) of world `(x, z)`.
  fn texel_coordinates(&self, x: f32, z: f32) -> [f32; 2] {
    let inverse = 1.0 / self.texel_metres;
    [(x + self.half[0]) * inverse, (z + self.half[1]) * inverse]
  }

  /// The texel nearest world `(x, z)`, clamped to the texture:
  /// `textureLoad` at the rounded texel in the shaders.
  pub fn nearest(&self, x: f32, z: f32) -> usize {
    let [tx, tz] = self.texel_coordinates(x, z);
    let clamp = |t: f32, size: u32| ((t + 0.5).floor() as i32).clamp(0, size as i32 - 1) as usize;
    clamp(tz, self.height) * self.width as usize + clamp(tx, self.width)
  }

  /// Bilinear interpolation of `value` at world `(x, z)`, clamped to the
  /// texture like `texel_height` in `ground.wgsl`.
  pub fn bilinear(&self, x: f32, z: f32, value: impl Fn(usize) -> f32) -> f32 {
    let [tx, tz] = self.texel_coordinates(x, z);
    let tx = tx.clamp(0.0, self.width as f32 - 1.001);
    let tz = tz.clamp(0.0, self.height as f32 - 1.001);
    let (bx, bz) = (tx.floor(), tz.floor());
    let (fx, fz) = (tx - bx, tz - bz);
    let index = bz as usize * self.width as usize + bx as usize;
    let row = self.width as usize;
    let top = value(index) + (value(index + 1) - value(index)) * fx;
    let bottom = value(index + row) + (value(index + row + 1) - value(index + row)) * fx;
    top + (bottom - top) * fz
  }

  /// The grass density mask's multiplier at world `(x, z)`, 0 to 2: its
  /// bytes read bilinearly, as `grass_generate.wgsl` samples them.
  pub fn grass_multiplier(&self, x: f32, z: f32) -> f32 {
    if self.grass_mask.is_empty() {
      return 1.0;
    }

    let byte = self.bilinear(x, z, |texel| f32::from(self.grass_mask[texel]));
    crate::terrain::painted::density_multiplier(byte.round() as u8)
  }

  /// Bilinear ground height at world `(x, z)`.
  pub fn height_at(&self, x: f32, z: f32) -> f32 {
    self.bilinear(x, z, |index| self.heights[index])
  }

  /// Whether world `(x, z)` is in a lake or river at least a sample wide,
  /// or within `clearance` metres of it. The field is 0 on water samples
  /// and measured from the edge half a sample out, so bilinearly it reads
  /// a quarter of a texel at the edge.
  pub fn in_water(&self, x: f32, z: f32, clearance: f32) -> bool {
    self.water_distance(x, z) < 0.25 * self.texel_metres + clearance
  }

  /// Bilinear distance to water at world `(x, z)`, in metres.
  pub fn water_distance(&self, x: f32, z: f32) -> f32 {
    self.bilinear(x, z, |index| f32::from(self.banks[index][0]))
      * (crate::render::water::WET_BANK_RANGE_METRES / 255.0)
  }

  /// Whether world `(x, z)` is on the terrain's footprint.
  pub fn on_map(&self, x: f32, z: f32) -> bool {
    x.abs() <= self.half[0] && z.abs() <= self.half[1]
  }

  /// Slope in degrees at world `(x, z)`, from the heights `SLOPE_SPAN`
  /// either side.
  pub fn slope_degrees(&self, x: f32, z: f32) -> f32 {
    let dx = self.height_at(x + SLOPE_SPAN, z) - self.height_at(x - SLOPE_SPAN, z);
    let dz = self.height_at(x, z + SLOPE_SPAN) - self.height_at(x, z - SLOPE_SPAN);
    let run = 2.0 * SLOPE_SPAN;
    (crate::maths::length2(dx, dz) / run).atan().to_degrees()
  }
}

/// Words per segment in [`channel_bins`].
pub const BIN_WORDS: usize = 11;

/// The drawn channels and plunge pools of `waters()` in `render/flora.rs`,
/// packed for the generators: the same grid, the same segments and the
/// same clearances. Words: origin x and z, cell size (as f32 bits),
/// columns and rows, then `columns x rows + 1` cell starts, then
/// [`BIN_WORDS`] floats per segment (a.x, a.z, b.x, b.z, half width at a
/// and at b, clearance, the bed stones' median size and chance, the
/// riparian band's density, and its scrub's: the same until
/// [`mask_scrub`] applies a tree density mask).
/// Without channels it is one empty cell.
pub fn channel_bins(
  map: &HeightMap,
  rivers: Option<&crate::render::water::RiverNetwork>,
) -> Vec<u32> {
  let metres = map.metadata.metres_per_sample.max(0.001);
  let half = [
    (map.metadata.width as f32 - 1.0) * metres * 0.5,
    (map.metadata.height as f32 - 1.0) * metres * 0.5,
  ];
  let Some(rivers) = rivers else {
    return vec![0, 0, 1f32.to_bits(), 1, 1, 0, 0];
  };
  let grid = crate::render::flora::waters(map, rivers, half);
  let (origin, cell, columns, rows, start, items) = grid.parts();

  if start.is_empty() {
    return vec![0, 0, 1f32.to_bits(), 1, 1, 0, 0];
  }

  let mut words = vec![
    origin[0].to_bits(),
    origin[1].to_bits(),
    cell.to_bits(),
    columns as u32,
    rows as u32,
  ];
  words.extend_from_slice(start);

  for water in items {
    words.extend(
      [
        water.a[0],
        water.a[1],
        water.b[0],
        water.b[1],
        water.half[0],
        water.half[1],
        water.clearance,
        water.stones[0],
        water.stones[1],
        water.band,
        water.band,
      ]
      .map(f32::to_bits),
    );
  }

  words
}

/// Scale the riparian scrub along each channel of `bins` (from
/// [`channel_bins`] over `map`) by the tree density `mask` (one byte per
/// sample, see `painted::density_multiplier`) at its middle, or restore
/// it without one. The band's tufts and herbs follow the grass mask
/// instead.
pub fn mask_scrub(bins: &mut [u32], map: &HeightMap, mask: Option<&[u8]>) {
  let first_item = 5 + (bins[3] * bins[4]) as usize + 1;
  let items = bins.len().saturating_sub(first_item) / BIN_WORDS;
  let metres = map.metadata.metres_per_sample.max(0.001);
  let half = [
    (map.metadata.width as f32 - 1.0) * metres * 0.5,
    (map.metadata.height as f32 - 1.0) * metres * 0.5,
  ];

  for item in 0..items {
    let base = first_item + item * BIN_WORDS;
    let float = |index: usize| f32::from_bits(bins[index]);
    let multiplier = mask.map_or(1.0, |mask| {
      let at = |a: usize, b: usize, size: u32, half: f32| {
        (((0.5 * (float(a) + float(b)) + half) / metres)
          .round()
          .max(0.0) as u32)
          .min(size.saturating_sub(1))
      };
      let x = at(base, base + 2, map.metadata.width, half[0]);
      let z = at(base + 1, base + 3, map.metadata.height, half[1]);
      let value = mask
        .get((z * map.metadata.width + x) as usize)
        .copied()
        .unwrap_or(128);
      crate::terrain::painted::density_multiplier(value)
    });
    bins[base + 10] = (float(base + 9) * multiplier).to_bits();
  }
}

/// Whether world `(x, z)` lies within a channel's water, plus its
/// clearance when `clearance` is set: the shaders' `in_channel`, line for
/// line, over the words of [`channel_bins`].
pub fn in_channel(bins: &[u32], x: f32, z: f32, clearance: bool) -> bool {
  let float = |index: usize| f32::from_bits(bins[index]);
  let (origin, cell) = ([float(0), float(1)], float(2));
  let (columns, rows) = (bins[3] as i32, bins[4] as i32);
  let cells = (columns * rows) as usize;
  let first_item = 5 + cells + 1;
  let cx = ((x - origin[0]) / cell).floor() as i32;
  let cz = ((z - origin[1]) / cell).floor() as i32;

  for row in (cz - 1).max(0)..=(cz + 1).min(rows - 1) {
    let (x0, x1) = ((cx - 1).max(0), (cx + 1).min(columns - 1));

    if x0 > x1 {
      continue;
    }

    let from = bins[5 + (row * columns + x0) as usize] as usize;
    let to = bins[5 + (row * columns + x1 + 1) as usize] as usize;

    for item in from..to {
      let base = first_item + item * BIN_WORDS;
      let water = Water {
        a: [float(base), float(base + 1)],
        b: [float(base + 2), float(base + 3)],
        half: [float(base + 4), float(base + 5)],
        clearance: if clearance { float(base + 6) } else { 0.0 },
        ..Water::default()
      };

      if water.intrusion(x, z) > 0.0 {
        return true;
      }
    }
  }

  false
}

/// The channel whose water's edge is nearest world `(x, z)`, within
/// `reach` metres outside it: how far outside its edge the point is
/// (negative in the water), and where its words start in `bins`. The
/// shaders' `near_channel`, line for line, over the words of
/// [`channel_bins`].
pub fn near_channel(bins: &[u32], x: f32, z: f32, reach: f32) -> Option<(f32, usize)> {
  let float = |index: usize| f32::from_bits(bins[index]);
  let (origin, cell) = ([float(0), float(1)], float(2));
  let (columns, rows) = (bins[3] as i32, bins[4] as i32);
  let first_item = 5 + (columns * rows) as usize + 1;
  let cx = ((x - origin[0]) / cell).floor() as i32;
  let cz = ((z - origin[1]) / cell).floor() as i32;
  let mut best: Option<(f32, usize)> = None;

  for row in (cz - 1).max(0)..=(cz + 1).min(rows - 1) {
    let (x0, x1) = ((cx - 1).max(0), (cx + 1).min(columns - 1));

    if x0 > x1 {
      continue;
    }

    let from = bins[5 + (row * columns + x0) as usize] as usize;
    let to = bins[5 + (row * columns + x1 + 1) as usize] as usize;

    for item in from..to {
      let base = first_item + item * BIN_WORDS;
      let water = Water {
        a: [float(base), float(base + 1)],
        b: [float(base + 2), float(base + 3)],
        half: [float(base + 4), float(base + 5)],
        ..Water::default()
      };
      let edge = -water.intrusion(x, z);

      if edge <= reach && best.is_none_or(|best| edge < best.0) {
        best = Some((edge, base));
      }
    }
  }

  best
}

/// The bed stones of the channel whose water's edge is nearest world `(x,
/// z)`, within `reach` metres outside it: how far outside its edge the
/// point is (negative in the water), and the stones' median size and
/// chance there, as `stone_candidate` in the generators reads them.
pub fn channel_stones(bins: &[u32], x: f32, z: f32, reach: f32) -> Option<[f32; 3]> {
  let (edge, base) = near_channel(bins, x, z, reach)?;
  Some([
    edge,
    f32::from_bits(bins[base + 7]),
    f32::from_bits(bins[base + 8]),
  ])
}

/// The unit direction (x, z) from world `(x, z)` to the nearest point on
/// a channel's centreline within [`LEAN_WATER_METRES`], if any: the
/// shaders' `channel_lean`, line for line, over the words of
/// [`channel_bins`].
pub fn channel_lean(bins: &[u32], x: f32, z: f32) -> Option<[f32; 2]> {
  let float = |index: usize| f32::from_bits(bins[index]);
  let (origin, cell) = ([float(0), float(1)], float(2));
  let (columns, rows) = (bins[3] as i32, bins[4] as i32);
  let first_item = 5 + (columns * rows) as usize + 1;
  let cx = ((x - origin[0]) / cell).floor() as i32;
  let cz = ((z - origin[1]) / cell).floor() as i32;
  let mut best = LEAN_WATER_METRES * LEAN_WATER_METRES;
  let mut towards = None;

  for row in (cz - 1).max(0)..=(cz + 1).min(rows - 1) {
    let (x0, x1) = ((cx - 1).max(0), (cx + 1).min(columns - 1));

    if x0 > x1 {
      continue;
    }

    let from = bins[5 + (row * columns + x0) as usize] as usize;
    let to = bins[5 + (row * columns + x1 + 1) as usize] as usize;

    for item in from..to {
      let base = first_item + item * BIN_WORDS;
      let (a, b) = (
        [float(base), float(base + 1)],
        [float(base + 2), float(base + 3)],
      );
      let d = [b[0] - a[0], b[1] - a[1]];
      let length = d[0] * d[0] + d[1] * d[1];
      let t = if length > 0.0 {
        (((x - a[0]) * d[0] + (z - a[1]) * d[1]) / length).clamp(0.0, 1.0)
      } else {
        0.0
      };
      let p = [a[0] + d[0] * t - x, a[1] + d[1] * t - z];
      let distance = p[0] * p[0] + p[1] * p[1];

      if distance < best && distance > 1e-6 {
        best = distance;
        let l = distance.sqrt();
        towards = Some([p[0] / l, p[1] / l]);
      }
    }
  }

  towards
}

/// A tree's shape bits: its variant, age class and lean (see
/// `render/flora.rs`), from its shape hash. `young` trees (seedlings,
/// understorey, dwarf shrubs) take the young age class and stunted ones
/// krummholz; by water the old give way to the mature. The lean points
/// over water within [`LEAN_WATER_METRES`] of a channel, and otherwise
/// downhill and with the prevailing wind, and grows with the slope. The
/// generators' `shape_bits` does the same.
pub fn shape_bits(
  ground: &GroundData,
  bins: &[u32],
  hash: u32,
  x: f32,
  z: f32,
  young: bool,
  stunted: bool,
) -> u32 {
  let water = channel_lean(bins, x, z);
  let age = if stunted {
    Age::Krummholz
  } else if young {
    Age::Young
  } else if hash >> 2 & 255 < OLD_SHARE && water.is_none() {
    Age::Old
  } else {
    Age::Mature
  };
  let dx = ground.height_at(x + SLOPE_SPAN, z) - ground.height_at(x - SLOPE_SPAN, z);
  let dz = ground.height_at(x, z + SLOPE_SPAN) - ground.height_at(x, z - SLOPE_SPAN);
  let run = 2.0 * SLOPE_SPAN;
  let slope = (crate::maths::length2(dx, dz) / run).atan().to_degrees();
  // Downhill, weighted by the slope, and with the prevailing wind.
  let fall = (crate::maths::length2(dx, dz) / run * 10.0).min(2.0);
  let downhill = crate::maths::length2(dx, dz).max(1e-6);
  let lean = water.unwrap_or([
    -dx / downhill * fall + PREVAILING_WIND[0] * 0.5,
    -dz / downhill * fall + PREVAILING_WIND[1] * 0.5,
  ]);
  let sector = (lean[0].atan2(lean[1]) / (std::f32::consts::TAU / 16.0)).round() as i32 & 15;
  let roll = hash >> 10 & 255;
  let mut strength = u32::from(slope >= 12.0) + u32::from(slope >= 25.0) + u32::from(slope >= 35.0);
  strength = strength.max(u32::from(roll >= 128) + u32::from(roll >= 218));

  if water.is_some() {
    strength = strength.max(2);
  }

  (hash & 3) << TREE_VARIANT_SHIFT
    | (age as u32) << TREE_AGE_SHIFT
    | (sector as u32 | strength.min(3) << 4) << TREE_LEAN_SHIFT
}

/// The rotation a stunted tree stands at: its model's windward side (+x)
/// facing into the prevailing wind, give or take 0.35 radians.
pub fn krummholz_rotation(hash: u32) -> f32 {
  PREVAILING_WIND[1].atan2(-PREVAILING_WIND[0]) + ((hash >> 18 & 255) as f32 / 255.0 - 0.5) * 0.7
}

/// What every tree point needs beyond the ground: the lattice seed and
/// `p`'s scale, species variation, and each species' steepest slope.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeRules {
  /// Lattice seed ([`crate::render::lattice::lattice_seed`]).
  pub seed: u32,
  /// `p` per unit of cover red squared: `target_density / 1100 x
  /// COVER_SHARE_MAX / 255^2` (see `flora::cover_share`).
  pub p_scale: f32,
  /// Per-tree size and colour variety, 0 to 1.
  pub variation: f32,
  /// Steepest slope each species grows on, in degrees.
  pub max_slopes: [f32; 8],
  /// The far set's share of each point's probability
  /// ([`crate::render::lattice::far_keep`]); the streamed tiles hold the
  /// rest.
  pub far_keep: f32,
  /// Ranks (relative to `p`) from this up are understorey: the trees a
  /// stand holds beyond the old maximum density grow beneath the canopy,
  /// not as more full-size trees. 1 up to density 1.
  pub understorey: f32,
  /// The boulders' lattice seed, when there are boulders to leave room
  /// for.
  pub boulders: Option<u32>,
}

impl TreeRules {
  /// The rules for flora options at an effective density `d` (0 to 4).
  pub fn new(options: &vista_types::FloraOptions, d: f32) -> Self {
    Self {
      seed: crate::render::lattice::lattice_seed(options.seed_offset),
      p_scale: crate::render::lattice::target_density(d)
        / crate::render::lattice::TREE_CANDIDATES_PER_HECTARE
        * crate::render::flora::cover_share(1),
      variation: options.species_variation.clamp(0.0, 1.0),
      max_slopes: std::array::from_fn(|species| {
        species_niche(TreeSpecies::ALL[species]).max_slope_degrees
      }),
      far_keep: crate::render::lattice::FAR_KEEP,
      understorey: (crate::render::lattice::target_density(1.0)
        / crate::render::lattice::target_density(d).max(1e-6))
      .min(1.0),
      boulders: None,
    }
  }
}

/// The tree at lattice point `(ix, iz)` whose rank relative to `p` is at
/// least `low` and below `high`, if one grows there: the far set takes
/// `[0, far_keep)`, the streamed tiles `[far_keep, 1)`. `tree_generate.wgsl`
/// decides each point the same way; its height is the ground under it
/// here, and the drawn mesh there.
pub fn tree_at(
  ground: &GroundData,
  bins: &[u32],
  rules: &TreeRules,
  ix: i32,
  iz: i32,
  low: f32,
  high: f32,
) -> Option<TreeInstance> {
  let hash = point_hash(ix, iz, rules.seed);
  let [x, z] = jittered(ix, iz, hash, TREE_PITCH);

  if !ground.on_map(x, z) {
    return None;
  }

  let texel = ground.nearest(x, z);
  let cover = ground.cover[texel];
  let red = f32::from(cover[0]);
  let p = red * red * rules.p_scale * crate::render::lattice::clump(ix, iz, rules.seed);
  let rank = unit(hash[2]);

  if cover[0] == 0 || rank < p * low || rank >= p * high {
    return None;
  }

  let traits = point_hash(ix, iz, rules.seed ^ TREE_TRAITS_SALT);
  let second = unit(traits[0]) < f32::from(cover[3] & 0x7f) / 127.0;
  let species = u32::from(if second { cover[2] } else { cover[1] }).min(7);
  let elevation = ground.height_at(x, z);

  if elevation <= ground.sea + TREE_WATER_LINE
    || ground.in_water(x, z, TREE_WATER_CLEARANCE)
    || ground.slope_degrees(x, z) >= rules.max_slopes[species as usize]
    || in_channel(bins, x, z, true)
    || rules.boulders.is_some_and(|seed| {
      crate::render::boulders::under_boulder(ground, bins, seed, x, z, TRUNK_CLEARANCE)
    })
  {
    return None;
  }

  let surface = ground.surface[texel];
  let (temperature, moisture) = (f32::from(surface[0]) / 255.0, f32::from(surface[1]) / 255.0);
  let biome = u32::from(surface[3]);
  let (size_roll, tint_roll) = (unit(traits[1]), unit(traits[2]));
  // Trees on poor ground and at forest edges grow smaller.
  let share = red * red * (crate::render::flora::COVER_SHARE_MAX / 65_025.0);
  let vigour = 0.75 + 0.3 * (share * 3.0).min(1.0);
  let mut scale = (1.0 + (size_roll - 0.5) * 0.55 * rules.variation.max(0.15)) * vigour;
  let mut tint = 0.5 + (tint_roll - 0.5) * rules.variation.max(0.1);
  let mut dryness = ((temperature - 0.45) * 1.5 + (0.5 - moisture) * 1.5).clamp(0.0, 1.0);

  let mut young = traits[1] & 255 < YOUNG_SHARE;

  if young {
    scale = 0.45 + 0.35 * size_roll;
  }

  // Understorey trees are a quarter to a half of full size, so a dense
  // stand is a canopy over saplings, not a wall of giant trunks.
  if rank / p >= rules.understorey {
    scale = 0.25 + 0.25 * size_roll;
    young = true;
  }

  let shrub = species == TreeSpecies::Shrub as u32;
  let tundra = biome == BiomeKind::IceArctic as u32;
  let stunted = !shrub && cover[3] & crate::render::flora::COVER_STUNTED != 0;

  // Tundra shrubs are dwarf willow and birch: knee to waist high (the
  // young shrub's dwarf form, 0.4 to 0.8 m here), and brown rather than
  // green for most of the year. Shrubs above the trees are as small.
  if shrub && (tundra || biome == BiomeKind::AlpineTransition as u32) {
    scale = 0.6 + 0.6 * size_roll;
    young = true;

    if tundra {
      tint = 0.15 + 0.2 * tint_roll;
      dryness = 0.75;
    }
  } else if stunted {
    scale *= 0.7;
  }

  let shape = point_hash(ix, iz, rules.seed ^ TREE_SHAPE_SALT)[0];
  let rotation = if stunted {
    krummholz_rotation(shape)
  } else {
    (traits[2] & 255) as f32 * (std::f32::consts::TAU / 256.0)
  };

  // Below `RANK_LEVELS`, so a stored rank always reads as under 1 and a
  // tree the whole far set holds never thins.
  let rank = ((rank / p * RANK_LEVELS as f32) as u32).min(RANK_LEVELS - 1);
  Some(TreeInstance {
    position: [x, elevation, z],
    scale,
    rotation,
    tint,
    species: species
      | TREE_GROUNDED
      | TREE_LATTICE
      | if stunted { TREE_STUNTED } else { 0 }
      | shape_bits(ground, bins, shape, x, z, young, stunted)
      | rank << TREE_RANK_SHIFT,
    dryness,
  })
}

/// Riparian scrub's chance per lattice point where its band's density is
/// 1, before its gaps and steep banks thin it.
pub const SCRUB_CHANCE: f32 = 0.45;
/// Scrub grows from this far outside a channel's edge, in metres, out to
/// 4 m plus half the channel's width.
pub const SCRUB_FROM: f32 = 1.0;
/// How far out a desert's narrow line of green reaches, in metres.
pub const SCRUB_DESERT: f32 = 2.5;

/// Pack a riparian scrub mix: species `a` with a share of `share`
/// fifteenths, the rest `b`.
const fn scrub(a: TreeSpecies, b: TreeSpecies, share: u32) -> u32 {
  a as u32 | (b as u32) << 4 | share << 8
}

/// Riparian scrub by biome, in [`BiomeKind`] order (see [`scrub`]); 0
/// where none grows. Temperate streams are lined with willow (the shrub)
/// and alder (young broadleaves); cold ones with willow alone; tropical
/// ones with broad-leaved shrubs and palms; swamps with shrubs and young
/// cypress. Above the trees, on tundra, ice and the bare peaks, only
/// sedge and herbs line the water. `tree_generate.wgsl` holds the same
/// words.
pub const SCRUB_SPECIES: [u32; 19] = {
  use TreeSpecies::{Cypress, Oak, Palm, Shrub};
  let willow = scrub(Shrub, Shrub, 15);
  [
    scrub(Shrub, Oak, 10),
    scrub(Shrub, Oak, 10),
    scrub(Shrub, Oak, 9),
    scrub(Shrub, Oak, 8),
    willow,
    willow,
    willow,
    0,
    willow,
    scrub(Shrub, Palm, 9),
    willow,
    scrub(Shrub, Palm, 9),
    scrub(Shrub, Palm, 8),
    scrub(Shrub, Cypress, 9),
    0,
    0,
    0,
    0,
    0,
  ]
};

/// The riparian scrub at lattice point `(ix, iz)` of its own lattice
/// (the trees' pitch, [`RIPARIAN_SALT`]), whose rank relative to its
/// chance lies in `[low, high)`, as [`tree_at`] ranks trees: shrubs and
/// young trees 2 to 6 m tall from 1 m to `4 + w / 2` m outside a drawn
/// channel's edge, by its band's density (`water::riparian_band`), in
/// clumps with gaps 10 to 40 m long where the water shows through,
/// thinning on banks steeper than 35 degrees, none on snow, and leaning
/// over the water. `tree_generate.wgsl` decides each point the same way.
pub fn scrub_at(
  ground: &GroundData,
  bins: &[u32],
  rules: &TreeRules,
  ix: i32,
  iz: i32,
  low: f32,
  high: f32,
) -> Option<TreeInstance> {
  let seed = rules.seed ^ RIPARIAN_SALT;
  let hash = point_hash(ix, iz, seed);
  let [x, z] = jittered(ix, iz, hash, TREE_PITCH);

  if !ground.on_map(x, z) {
    return None;
  }

  let (edge, base) = near_channel(bins, x, z, 1e9)?;
  let float = |index: usize| f32::from_bits(bins[index]);
  let texel = ground.nearest(x, z);
  let surface = ground.surface[texel];
  let mix = SCRUB_SPECIES
    .get(usize::from(surface[3]))
    .copied()
    .unwrap_or(0);
  let outer = if surface[3] == BiomeKind::SavannahExpanse as u8 {
    SCRUB_DESERT
  } else {
    4.0 + 0.5 * (float(base + 4) + float(base + 5))
  };

  if mix == 0 || edge < SCRUB_FROM || edge > outer || surface[2] > 127 {
    return None;
  }

  let gaps = ((crate::render::lattice::clump(ix, iz, seed) - 0.15) / 0.5).clamp(0.0, 1.0);
  let steep = ((45.0 - ground.slope_degrees(x, z)) / 10.0).clamp(0.0, 1.0);
  let chance = SCRUB_CHANCE * float(base + 10) * gaps * steep;
  let rank = unit(hash[2]);

  if chance <= 0.0
    || rank < chance * low
    || rank >= chance * high
    || ground.height_at(x, z) <= ground.sea + TREE_WATER_LINE
    // The wet-bank field is coarse beside a stream narrower than a
    // sample: scrub keeps out of the channels by their drawn edges, and
    // out of water only where the field is water all round.
    || ground.water_distance(x, z) <= 0.0
    || in_channel(bins, x, z, true)
    || rules.boulders.is_some_and(|seed| {
      crate::render::boulders::under_boulder(ground, bins, seed, x, z, TRUNK_CLEARANCE)
    })
  {
    return None;
  }

  let traits = point_hash(ix, iz, seed ^ TREE_TRAITS_SALT);
  // Cold streams are lined with willow alone.
  let first = surface[0] < 97 || unit(traits[0]) * 15.0 < ((mix >> 8) & 15) as f32;
  let species = if first { mix & 15 } else { (mix >> 4) & 15 };
  let shrub = species == TreeSpecies::Shrub as u32;
  let size_roll = unit(traits[1]);
  let scale = if shrub {
    1.0 + 1.3 * size_roll
  } else {
    0.15 + 0.15 * size_roll
  };
  let shape = point_hash(ix, iz, seed ^ TREE_SHAPE_SALT)[0];
  let rank = ((rank / chance * RANK_LEVELS as f32) as u32).min(RANK_LEVELS - 1);
  Some(TreeInstance {
    position: [x, ground.height_at(x, z), z],
    scale,
    rotation: (traits[2] & 255) as f32 * (std::f32::consts::TAU / 256.0),
    tint: 0.35 + 0.3 * unit(traits[2]),
    species: species
      | TREE_GROUNDED
      | TREE_LATTICE
      | shape_bits(ground, bins, shape, x, z, !shrub, false)
      | rank << TREE_RANK_SHIFT,
    dryness: 0.0,
  })
}

/// Expected riparian plants per texel at every rank, from the drawn
/// `channels` (runs of x, z and half width) and their `bands` (see
/// `water::riparian_band`): `per_side(half)` gives the plants per metre of
/// a bank where the band's density is 1, and how far from the centreline
/// they stand. Each stretch's share lands in the texels beside it on
/// either bank.
pub fn band_mass(
  ground: &GroundData,
  channels: &[Vec<[f32; 3]>],
  bands: &[Vec<f32>],
  per_side: &dyn Fn(f32) -> (f32, f32),
) -> Vec<f32> {
  let mut mass = vec![0.0; (ground.width * ground.height) as usize];

  if mass.is_empty() {
    return mass;
  }

  for (run, bands) in channels
    .iter()
    .zip(bands)
    .filter(|(run, bands)| run.len() == bands.len())
  {
    for (pair, band) in run.windows(2).zip(bands.windows(2)) {
      let (dx, dz) = (pair[1][0] - pair[0][0], pair[1][1] - pair[0][1]);
      let length = crate::maths::length2(dx, dz);
      let (per_metre, reach) = per_side(0.5 * (pair[0][2] + pair[1][2]));
      let each = length * per_metre * 0.5 * (band[0] + band[1]);
      let across = [-dz / length.max(1e-6), dx / length.max(1e-6)];

      for side in [-1.0, 1.0] {
        let texel = ground.nearest(
          0.5 * (pair[0][0] + pair[1][0]) + side * reach * across[0],
          0.5 * (pair[0][1] + pair[1][1]) + side * reach * across[1],
        );
        mass[texel] += each;
      }
    }
  }

  mass
}

/// Expected riparian scrub per texel at every rank (see [`band_mass`]).
pub fn scrub_mass(ground: &GroundData, channels: &[Vec<[f32; 3]>], bands: &[Vec<f32>]) -> Vec<f32> {
  band_mass(ground, channels, bands, &|half| {
    let width = 4.0 + half - SCRUB_FROM;
    (
      width * SCRUB_CHANCE / (TREE_PITCH * TREE_PITCH),
      half + SCRUB_FROM + 0.5 * width,
    )
  })
}

/// The lattice points riparian scrub may stand on with a rank relative to
/// its chance below `high` (inside `region`, `[min_x, min_z, max_x,
/// max_z]`, when given), each once, in order: every point [`scrub_at`]
/// takes passes this cheaper test against the channel nearest it, since
/// its chance there is at most this.
fn scrub_points(
  bins: &[u32],
  rules: &TreeRules,
  region: Option<[f32; 4]>,
  high: f32,
) -> Vec<(i32, i32)> {
  let float = |index: usize| f32::from_bits(bins[index]);
  let first_item = 5 + (bins[3] * bins[4]) as usize + 1;
  let items = bins.len().saturating_sub(first_item) / BIN_WORDS;
  let seed = rules.seed ^ RIPARIAN_SALT;
  let mut points = Vec::new();

  for item in 0..items {
    let base = first_item + item * BIN_WORDS;
    let water = Water {
      a: [float(base), float(base + 1)],
      b: [float(base + 2), float(base + 3)],
      half: [float(base + 4), float(base + 5)],
      ..Water::default()
    };
    let most = SCRUB_CHANCE * float(base + 10) * high;
    let outer = 4.0 + 0.5 * (water.half[0] + water.half[1]);

    if most <= 0.0 {
      continue;
    }

    // Points stray from their cells by the jitter.
    let reach = outer + water.half[0].max(water.half[1]) + TREE_PITCH;
    let mut low = [
      water.a[0].min(water.b[0]) - reach,
      water.a[1].min(water.b[1]) - reach,
    ];
    let mut top = [
      water.a[0].max(water.b[0]) + reach,
      water.a[1].max(water.b[1]) + reach,
    ];

    if let Some(region) = region {
      low = [
        low[0].max(region[0] - TREE_PITCH),
        low[1].max(region[1] - TREE_PITCH),
      ];
      top = [
        top[0].min(region[2] + TREE_PITCH),
        top[1].min(region[3] + TREE_PITCH),
      ];
    }

    let cell = |at: f32| (at / TREE_PITCH).floor() as i32;

    for iz in cell(low[1])..=cell(top[1]) {
      for ix in cell(low[0])..=cell(top[0]) {
        // The cheapest tests first: most points fail the rank alone. A hair
        // over, so rounding never drops a point `scrub_at` takes.
        let hash = point_hash(ix, iz, seed);
        let rank = unit(hash[2]);

        if rank >= most * 1.0001 {
          continue;
        }

        let [x, z] = jittered(ix, iz, hash, TREE_PITCH);
        let edge = -water.intrusion(x, z);

        if (SCRUB_FROM..=outer).contains(&edge)
          && rank
            < most
              * ((crate::render::lattice::clump(ix, iz, seed) - 0.15) / 0.5).clamp(0.0, 1.0)
              * 1.0001
        {
          points.push(point_key(ix, iz));
        }
      }
    }
  }

  points.sort_unstable();
  points.dedup();
  points
    .into_iter()
    .map(|key| ((key & 0xffff) as i32 - 32_768, (key >> 16) as i32 - 32_768))
    .collect()
}

/// A lattice point as one word, row by row: 16 bits a side reach 98 km
/// either way at the trees' pitch, and sorting words keeps one sort in the
/// binary.
fn point_key(ix: i32, iz: i32) -> usize {
  let half = |i: i32| (i + 32_768).clamp(0, 0xffff) as usize;
  half(iz) << 16 | half(ix)
}

/// Every lattice tree over the map with a rank relative to `p` below
/// `keep`: the far set with `keep = far_keep`, or every tree with 1. Only
/// the lattice points near texels with cover are read, so bare ground
/// costs nothing. At most `max` are kept: those of lowest rank.
pub fn lattice_trees(
  ground: &GroundData,
  bins: &[u32],
  rules: &TreeRules,
  keep: f32,
  max: usize,
) -> Vec<TreeInstance> {
  let mut trees = Vec::new();

  if ground.cover.len() != (ground.width * ground.height) as usize || rules.p_scale <= 0.0 {
    return trees;
  }

  let reach = ground.texel_metres * 0.5 + TREE_PITCH;

  for (texel, cover) in ground.cover.iter().enumerate() {
    if cover[0] == 0 {
      continue;
    }

    let (tx, tz) = (texel as u32 % ground.width, texel as u32 / ground.width);
    let (cx, cz) = (
      tx as f32 * ground.texel_metres - ground.half[0],
      tz as f32 * ground.texel_metres - ground.half[1],
    );
    let range = |c: f32| {
      (
        ((c - reach) / TREE_PITCH).floor() as i32,
        ((c + reach) / TREE_PITCH).floor() as i32,
      )
    };
    let ((x0, x1), (z0, z1)) = (range(cx), range(cz));

    for iz in z0..=z1 {
      for ix in x0..=x1 {
        // Each point belongs to the texel nearest its jittered position.
        let [x, z] = jittered(ix, iz, point_hash(ix, iz, rules.seed), TREE_PITCH);

        if ground.nearest(x, z) != texel {
          continue;
        }

        if let Some(tree) = tree_at(ground, bins, rules, ix, iz, 0.0, keep) {
          trees.push(tree);
        }
      }
    }
  }

  trees.extend(
    scrub_points(bins, rules, None, keep)
      .into_iter()
      .filter_map(|(ix, iz)| scrub_at(ground, bins, rules, ix, iz, 0.0, keep)),
  );

  if trees.len() > max {
    let rank = |tree: &TreeInstance| (tree.species >> TREE_RANK_SHIFT) as f64;
    let mut ranks: Vec<f64> = trees.iter().map(rank).collect();
    let cut = crate::terrain::stream_power::nth_value(&mut ranks, max.saturating_sub(1));
    trees.retain(|tree| rank(tree) <= cut);
    trees.truncate(max);
  }

  trees
}

/// Every lattice tree, of every rank, whose trunk stands within
/// `region` (`[min_x, min_z, max_x, max_z]` in metres, inclusive): the
/// far set and every streamed tile together, as [`lattice_trees`] gives
/// them with `keep = 1`, and in its order: texel by texel, skipping bare
/// ground. More than `max` trees is an error holding `max`, found before
/// storing the one over.
pub fn region_trees(
  ground: &GroundData,
  bins: &[u32],
  rules: &TreeRules,
  region: [f32; 4],
  max: usize,
) -> Result<Vec<TreeInstance>, usize> {
  let mut trees = Vec::new();

  if ground.cover.len() != (ground.width * ground.height) as usize || rules.p_scale <= 0.0 {
    return Ok(trees);
  }

  // The texels whose points can land in the region: each point belongs
  // to the texel nearest it.
  let texels = |low: f32, high: f32, half: f32, size: u32| {
    let texel = |at: f32| ((at.clamp(-half, half) + half) / ground.texel_metres).round() as u32;
    texel(low).min(size - 1)..=texel(high).min(size - 1)
  };
  let reach = ground.texel_metres * 0.5 + TREE_PITCH;

  for tz in texels(region[1], region[3], ground.half[1], ground.height) {
    for tx in texels(region[0], region[2], ground.half[0], ground.width) {
      let texel = (tz * ground.width + tx) as usize;

      if ground.cover[texel][0] == 0 {
        continue;
      }

      let (cx, cz) = (
        tx as f32 * ground.texel_metres - ground.half[0],
        tz as f32 * ground.texel_metres - ground.half[1],
      );
      let range = |c: f32| {
        ((c - reach) / TREE_PITCH).floor() as i32..=((c + reach) / TREE_PITCH).floor() as i32
      };

      for iz in range(cz) {
        for ix in range(cx) {
          let [x, z] = jittered(ix, iz, point_hash(ix, iz, rules.seed), TREE_PITCH);

          if ground.nearest(x, z) != texel
            || x < region[0]
            || x > region[2]
            || z < region[1]
            || z > region[3]
          {
            continue;
          }

          let Some(tree) = tree_at(ground, bins, rules, ix, iz, 0.0, 1.0) else {
            continue;
          };

          if trees.len() == max {
            return Err(max);
          }

          trees.push(tree);
        }
      }
    }
  }

  for (ix, iz) in scrub_points(bins, rules, Some(region), 1.0) {
    let Some(tree) = scrub_at(ground, bins, rules, ix, iz, 0.0, 1.0) else {
      continue;
    };
    let [x, _, z] = tree.position;

    if x < region[0] || x > region[2] || z < region[1] || z > region[3] {
      continue;
    }

    if trees.len() == max {
      return Err(max);
    }

    trees.push(tree);
  }

  Ok(trees)
}

/// The first lattice index of tile `tile` along one axis: points with
/// `index x pitch` in `[tile x size, (tile + 1) x size)` belong to it, in
/// whole units (metres for trees, centimetres for grass). `tile_first`
/// in the generators.
pub fn tile_first(tile: i32, size: i32, pitch: i32) -> i32 {
  let a = tile * size;
  a / pitch + i32::from(a % pitch > 0)
}

/// The trees one streamed tile holds: points with ranks in `[far_keep,
/// keep)`, as `tree_generate.wgsl` fills it, then its riparian scrub.
pub fn tile_trees(
  ground: &GroundData,
  bins: &[u32],
  rules: &TreeRules,
  tile: [i32; 2],
  keep: f32,
) -> Vec<TreeInstance> {
  let size = crate::render::lattice::TREE_TILE_METRES as i32;
  let pitch = TREE_PITCH as i32;
  let range = |t: i32| tile_first(t, size, pitch)..tile_first(t + 1, size, pitch);
  let cells: Vec<(i32, i32)> = range(tile[1])
    .flat_map(|iz| range(tile[0]).map(move |ix| (ix, iz)))
    .collect();
  let trees = cells
    .iter()
    .filter_map(|&(ix, iz)| tree_at(ground, bins, rules, ix, iz, rules.far_keep, keep));
  let scrub = cells
    .iter()
    .filter_map(|&(ix, iz)| scrub_at(ground, bins, rules, ix, iz, rules.far_keep, keep));
  trees.chain(scrub).collect()
}

/// The share of a tent of half-width 1 centred on 0 (a texel's weight
/// under bilinear interpolation, in texels) that lies between `a` and
/// `b`.
fn tent_share(a: f32, b: f32) -> f32 {
  let below = |u: f32| {
    let u = u.clamp(-1.0, 1.0);

    if u < 0.0 {
      (u + 1.0) * (u + 1.0) * 0.5
    } else {
      1.0 - (1.0 - u) * (1.0 - u) * 0.5
    }
  };
  below(b) - below(a)
}

/// Expected trees or tufts per tile at full density (every rank), over
/// the tiles of the map, and which tiles are worth generating.
#[derive(Clone, Debug, Default)]
pub struct TileMass {
  /// Tile edge in metres.
  pub tile: f32,
  /// Tile coordinates of the first column and row.
  first: [i32; 2],
  columns: i32,
  rows: i32,
  mass: Vec<f32>,
  wanted: Vec<bool>,
}

/// Most tiles a [`TileMass`] covers: 64 Mi tiles, 320 MiB of masses and
/// flags. The grid spans the whole map, so 16 m grass tiles over a map
/// 245 km across (2048 samples at 120 m) would need 1.2 GB; a layer whose
/// grid would pass this is not streamed (see [`TileMass::fits`]), and the
/// terrain's metadata says so.
pub const MAX_TILES: u64 = 64 << 20;

impl TileMass {
  /// Tile coordinates of the first column and row of tiles of edge `tile`
  /// over `ground`, and how many columns and rows there are.
  fn grid(ground: &GroundData, tile: f32) -> ([i32; 2], i32, i32) {
    let first = [
      (-ground.half[0] / tile).floor() as i32,
      (-ground.half[1] / tile).floor() as i32,
    ];
    let columns = (ground.half[0] / tile).floor() as i32 - first[0] + 1;
    let rows = (ground.half[1] / tile).floor() as i32 - first[1] + 1;
    (first, columns, rows)
  }

  /// Whether tiles of edge `tile` over `ground` number at most
  /// [`MAX_TILES`].
  pub fn fits(ground: &GroundData, tile: f32) -> bool {
    let (_, columns, rows) = Self::grid(ground, tile);
    columns.max(0) as u64 * rows.max(0) as u64 <= MAX_TILES
  }

  /// Sum `per_texel(texel)` (expected plants in a texel) over tiles of
  /// edge `tile` by texel centre; tiles within `spill` metres of a texel
  /// with plants are wanted, since jittered points reach across. Over
  /// more than [`MAX_TILES`] tiles, the mass is empty: nothing to stream.
  // `dyn` rather than generic: one copy in the binary serves every kind.
  pub fn build(
    ground: &GroundData,
    tile: f32,
    spill: f32,
    per_texel: &dyn Fn(usize) -> f32,
  ) -> Self {
    if !Self::fits(ground, tile) {
      return Self {
        tile,
        ..Self::default()
      };
    }

    let (first, columns, rows) = Self::grid(ground, tile);
    let mut mass = vec![0.0; (columns.max(0) * rows.max(0)) as usize];
    let mut wanted = vec![false; mass.len()];
    let index = |tx: i32, tz: i32| {
      let (x, z) = (tx - first[0], tz - first[1]);
      (x >= 0 && z >= 0 && x < columns && z < rows).then(|| (z * columns + x) as usize)
    };
    let reach = ground.texel_metres * 0.5 + spill;

    for texel in 0..(ground.width * ground.height) as usize {
      let value = per_texel(texel);

      if value <= 0.0 {
        continue;
      }

      let (x, z) = (
        (texel as u32 % ground.width) as f32 * ground.texel_metres - ground.half[0],
        (texel as u32 / ground.width) as f32 * ground.texel_metres - ground.half[1],
      );

      // The generators read the textures between texels, so a texel's
      // plants spread under its bilinear footprint, a tent two texels
      // wide: share them among the tiles it overlaps. Lumping each into
      // the tile holding its centre made a 16 m tile over four 12 m
      // texel centres read twice as full as it could be, and every slot
      // is sized for the fullest tile.
      let texel = ground.texel_metres.max(1e-3);
      let share = |centre: f32, t: i32| {
        let low = t as f32 * tile;
        tent_share((low - centre) / texel, (low + tile - centre) / texel)
      };
      let (x0, x1) = (
        ((x - texel) / tile).floor() as i32,
        ((x + texel) / tile).floor() as i32,
      );
      let (z0, z1) = (
        ((z - texel) / tile).floor() as i32,
        ((z + texel) / tile).floor() as i32,
      );

      for tz in z0..=z1 {
        let across_z = share(z, tz);

        for tx in x0..=x1 {
          if let Some(slot) = index(tx, tz) {
            mass[slot] += value * share(x, tx) * across_z;
          }
        }
      }

      for tz in ((z - reach) / tile).floor() as i32..=((z + reach) / tile).floor() as i32 {
        for tx in ((x - reach) / tile).floor() as i32..=((x + reach) / tile).floor() as i32 {
          if let Some(slot) = index(tx, tz) {
            wanted[slot] = true;
          }
        }
      }
    }

    Self {
      tile,
      first,
      columns,
      rows,
      mass,
      wanted,
    }
  }

  /// The trees of every rank the cover texture puts in each 64 m tile,
  /// and the riparian scrub of `scrub` (see [`scrub_mass`]; empty for
  /// none).
  pub fn trees(ground: &GroundData, rules: &TreeRules, scrub: &[f32]) -> Self {
    let area = ground.texel_metres * ground.texel_metres / (TREE_PITCH * TREE_PITCH);
    Self::build(
      ground,
      crate::render::lattice::TREE_TILE_METRES,
      TREE_PITCH,
      &|texel| {
        let red = f32::from(ground.cover.get(texel).map_or(0, |cover| cover[0]));
        let scrub = if rules.p_scale > 0.0 {
          scrub.get(texel).copied().unwrap_or(0.0)
        } else {
          0.0
        };
        (red * red * rules.p_scale).min(1.0) * area + scrub
      },
    )
  }

  /// The tufts of every rank in each 16 m tile, from the grass texture,
  /// and the riparian tufts and herbs of `riparian` (see
  /// `grass::riparian_mass`; empty for none).
  pub fn grass(
    ground: &GroundData,
    rules: &crate::render::grass::GrassRules,
    riparian: &[f32],
  ) -> Self {
    use crate::render::lattice::GRASS_PITCH;
    let area = ground.texel_metres * ground.texel_metres / (GRASS_PITCH * GRASS_PITCH);
    Self::build(
      ground,
      crate::render::lattice::GRASS_TILE_METRES,
      GRASS_PITCH,
      &|texel| {
        let grass = ground.grass.get(texel).copied().unwrap_or([0; 4]);
        let accept =
          f32::from(grass[0]).max(f32::from(grass[3]) * crate::render::grass::FLOOR_DENSITY);
        let mask = ground.grass_mask.get(texel).map_or(1.0, |value| {
          crate::terrain::painted::density_multiplier(*value)
        });
        (accept / 255.0 * rules.probability * mask).min(1.0) * area
          + riparian.get(texel).copied().unwrap_or(0.0) * mask
      },
    )
  }

  fn slot(&self, tx: i32, tz: i32) -> Option<usize> {
    let (x, z) = (tx - self.first[0], tz - self.first[1]);
    (x >= 0 && z >= 0 && x < self.columns && z < self.rows).then(|| (z * self.columns + x) as usize)
  }

  /// Plants expected in tile `(tx, tz)` at full density.
  pub fn at(&self, tx: i32, tz: i32) -> f32 {
    self.slot(tx, tz).map_or(0.0, |slot| self.mass[slot])
  }

  /// Whether tile `(tx, tz)` has any plants to generate.
  pub fn wanted(&self, tx: i32, tz: i32) -> bool {
    self.slot(tx, tz).is_some_and(|slot| self.wanted[slot])
  }

  /// Plants expected over the whole map at full density.
  pub fn total(&self) -> f32 {
    self.mass.iter().sum()
  }

  /// The most any tile holds.
  pub fn most(&self) -> f32 {
    self.mass.iter().copied().fold(0.0, f32::max)
  }

  /// Every tile with plants: coordinates and plants.
  pub fn tiles(&self) -> impl Iterator<Item = ([i32; 2], f32)> + '_ {
    self
      .mass
      .iter()
      .enumerate()
      .filter(|(_, mass)| **mass > 0.0)
      .map(|(slot, mass)| {
        let slot = slot as i32;
        (
          [
            self.first[0] + slot % self.columns,
            self.first[1] + slot / self.columns,
          ],
          *mass,
        )
      })
  }
}

/// The near radius the budget allows: it shrinks in steps of 10 % while
/// the drawn estimate is over budget, never below `floor`, and grows back
/// a step after 2 s with room to spare.
#[derive(Clone, Debug, PartialEq)]
pub struct DetailRadius {
  /// The radius in use, in metres.
  pub radius: f32,
  most: f32,
  floor: f32,
  calm: f32,
}

/// Seconds with room to spare before the radius grows back a step.
const REGROW_SECONDS: f32 = 2.0;

impl DetailRadius {
  /// A radius starting at its most.
  pub fn new(most: f32, floor: f32) -> Self {
    Self {
      radius: most,
      most,
      floor: floor.min(most),
      calm: 0.0,
    }
  }

  /// Change the largest radius, keeping the current one within it.
  pub fn set_most(&mut self, most: f32, floor: f32) {
    if (most - self.most).abs() > 1e-3 {
      *self = Self::new(most, floor);
    }
  }

  /// Judge this frame's `estimate` of plants drawn against `budget`, `dt`
  /// seconds after the last, and return the radius to use.
  pub fn update(&mut self, estimate: u32, budget: u32, dt: f32) -> f32 {
    if estimate > budget {
      self.radius = (self.radius * 0.9).max(self.floor);
      self.calm = 0.0;
    } else if self.radius < self.most && (estimate as f32) < budget as f32 * 0.8 * 0.81 {
      // A step out draws about 1 / 0.81 times as many: room to spare means
      // it would still fit.
      self.calm += dt;

      if self.calm >= REGROW_SECONDS {
        self.radius = (self.radius / 0.9).min(self.most);
        self.calm = 0.0;
      }
    } else {
      self.calm = 0.0;
    }

    self.radius
  }
}

/// One kind of streamed plant: its slots, which tile is in each, and how
/// many plants each tile holds.
#[derive(Clone, Debug)]
pub struct Stream {
  /// How the pool is divided into slots.
  pub layout: crate::render::lattice::TileLayout,
  /// Which tile sits in each slot.
  pub ring: crate::render::lattice::TileRing,
  /// Plants per tile.
  pub mass: TileMass,
  /// The fewest candidates thinning keeps (the far set's share for
  /// trees, 0 for grass).
  pub floor: f32,
  /// The full-density radius the slots are sized for: a larger radius
  /// would keep more in each tile than its slot holds. Unlimited where
  /// the slots do not depend on it.
  pub radius: f32,
  /// Whether plants hand over to the ground between
  /// [`crate::render::grass::handover_start`] and the view distance, as
  /// grass does.
  pub hands_over: bool,
}

/// Streamed trees fill tiles out to where thinning leaves only the far
/// set: `radius / sqrt(far_keep)`. Each slot holds a tile's non-far
/// trees, with room for clumps half as dense again as the densest tile.
pub fn tree_stream(mass: TileMass, radius: f32, cap: u32, far_keep: f32) -> Stream {
  use crate::render::lattice::{count_bound, TileLayout, TileRing, TREE_TILE_METRES};
  let capacity = count_bound(1.5 * mass.most() * (1.0 - far_keep));
  let mut radius = radius;
  let mut layout = TileLayout::new(TREE_TILE_METRES, &[radius / far_keep.sqrt()], &|_| capacity);

  // Over the generation cap, the tiles reach less far.
  while layout.instance_count() > cap && radius > 60.0 {
    radius *= 0.9;
    layout = TileLayout::new(TREE_TILE_METRES, &[radius / far_keep.sqrt()], &|_| capacity);
  }

  Stream {
    ring: TileRing::new(&layout),
    layout,
    mass,
    floor: far_keep,
    radius: f32::MAX,
    hands_over: false,
  }
}

/// Streamed grass fills tiles out to the view distance, in eight
/// distance classes: full density within `radius`, then classes in equal
/// ratios out to the view distance, whose slots hold only the tufts that
/// thinning and the handover keep at their inner edge (see
/// [`grass_keep`]). Over `cap` tufts in all, the radius comes in, to
/// [`GRASS_FLOOR_METRES`] at least (or `radius`, if smaller): the tufts
/// beyond it are widened to keep the cover, so full cover still reaches
/// the handover.
pub fn grass_stream(mass: TileMass, radius: f32, view: f32, cap: u32) -> Stream {
  use crate::render::lattice::{count_bound, TileLayout, TileRing, GRASS_TILE_METRES};
  let most = mass.most();
  let layout_for = |radius: f32| {
    let first = radius.min(view);
    let ratio = (view / first.max(1.0)).powf(1.0 / 7.0);
    let mut reaches = vec![first];

    while reaches.len() < 8 && reaches[reaches.len() - 1] < view {
      let next = first * ratio.powi(reaches.len() as i32);
      reaches.push(if reaches.len() == 7 {
        view
      } else {
        next.min(view)
      });
    }

    TileLayout::new(GRASS_TILE_METRES, &reaches, &|inner| {
      count_bound(most * grass_keep(inner, radius, view))
    })
  };
  let floor = GRASS_FLOOR_METRES.min(radius);
  let mut radius = radius;
  let mut layout = layout_for(radius);

  while layout.instance_count() > cap && radius > floor {
    radius = (radius * 0.95).max(floor);
    layout = layout_for(radius);
  }

  Stream {
    ring: TileRing::new(&layout),
    layout,
    mass,
    floor: 0.0,
    radius,
    hands_over: true,
  }
}

/// The share of a tile's tufts (by rank) generated `distance` metres from
/// the camera, for a full-cover radius `radius` and grass view distance
/// `view`: those thinning keeps, and of them the share the handover keeps
/// (see [`crate::render::grass::tuft_share`]). The cull draws no more.
pub fn grass_keep(distance: f32, radius: f32, view: f32) -> f32 {
  let t = crate::render::lattice::thinning(distance, radius, 0.0);
  let start = crate::render::grass::handover_start(radius, view);
  ((t.keep + t.band) * crate::render::grass::tuft_share(distance, start, view)).min(1.0)
}

/// Whether a sphere is at least partly inside the frustum `planes`.
pub fn in_frustum(planes: &[[f32; 4]; 6], centre: [f32; 3], radius: f32) -> bool {
  planes.iter().all(|plane| {
    plane[0] * centre[0] + plane[1] * centre[1] + plane[2] * centre[2] + plane[3] >= -radius
  })
}

impl Stream {
  /// The distance within which about `meshes` trees stand in view, at the
  /// share thinning keeps: the mesh distance that holds the full-mesh
  /// trees to that many, nearest first. `None` when fewer stand within
  /// `most` metres.
  pub fn mesh_reach(
    &self,
    ground: &GroundData,
    camera: [f32; 3],
    planes: &[[f32; 4]; 6],
    radius: f32,
    most: f32,
    meshes: u32,
  ) -> Option<f32> {
    let tile = self.mass.tile;
    let (cx, cz) = (camera[0] / tile, camera[2] / tile);
    let span = (most / tile).ceil() as i32 + 1;
    // Trees by distance, in half-tile rings: no sort needed.
    let ring = tile * 0.5;
    let last = ((most + tile) / ring) as usize;
    let mut rings = vec![0.0f32; last + 1];

    for tz in cz.floor() as i32 - span..=cz.floor() as i32 + span {
      for tx in cx.floor() as i32 - span..=cx.floor() as i32 + span {
        let mass = self.mass.at(tx, tz);
        let centre = [(tx as f32 + 0.5) * tile, (tz as f32 + 0.5) * tile];
        let height = ground.height_at(centre[0], centre[1]);
        let distance = crate::maths::length2(centre[0] - camera[0], centre[1] - camera[2]);

        if mass > 0.0
          && distance <= most + tile
          && in_frustum(planes, [centre[0], height, centre[1]], tile * 0.75 + 30.0)
        {
          let share = crate::render::lattice::thinning(distance, radius, self.floor).share;
          rings[((distance / ring) as usize).min(last)] += mass * share;
        }
      }
    }

    let mut total = 0.0;

    // The budget runs out part way through one ring.
    for (index, trees) in rings.into_iter().enumerate() {
      if total + trees >= meshes as f32 {
        let part = (meshes as f32 - total) / trees.max(1e-3);
        return Some(((index as f32 + part) * ring).max(tile * 0.25));
      }

      total += trees;
    }

    None
  }

  /// The plants expected to be drawn this frame, from each tile's mass,
  /// the share thinning keeps at its distance, and the frustum. Trees
  /// fade out between 0.8 and 1 times `canopy` (pass infinity for grass);
  /// beyond `view` nothing is drawn.
  #[allow(clippy::too_many_arguments)]
  pub fn estimate(
    &self,
    ground: &GroundData,
    camera: [f32; 3],
    planes: &[[f32; 4]; 6],
    radius: f32,
    view: f32,
    canopy: f32,
    live_only: bool,
  ) -> u32 {
    let tile = self.mass.tile;
    let visible = |coords: [i32; 2], mass: f32| {
      let centre = [
        (coords[0] as f32 + 0.5) * tile,
        0.0,
        (coords[1] as f32 + 0.5) * tile,
      ];
      let centre = [centre[0], ground.height_at(centre[0], centre[2]), centre[2]];
      let distance = crate::maths::length2(centre[0] - camera[0], centre[2] - camera[2]);

      if distance > view.min(canopy) + tile || !in_frustum(planes, centre, tile * 0.75 + 30.0) {
        return 0.0;
      }

      let fade = ((canopy - distance) / (0.2 * canopy)).clamp(0.0, 1.0);
      let thinned = crate::render::lattice::thinning(distance, radius, self.floor);
      let handover = if self.hands_over {
        let start = crate::render::grass::handover_start(radius, view);
        crate::render::grass::tuft_share(distance, start, view)
      } else {
        1.0
      };
      mass * thinned.share * fade * handover
    };
    let total: f32 = if live_only {
      self
        .ring
        .live()
        .map(|(_, coords)| visible(coords, self.mass.at(coords[0], coords[1])))
        .sum()
    } else {
      self
        .mass
        .tiles()
        .map(|(coords, mass)| visible(coords, mass))
        .sum()
    };
    total.round() as u32
  }
}

/// The grass's full-cover radius at most: `grassDetailMetres`, never past
/// the view distance.
pub fn grass_radius(budget: &vista_types::VegetationBudget, view: f32) -> f32 {
  budget.grass_detail_metres.min(view)
}

/// Streamed trees, grass and boulders, the radii their budgets allow, and
/// the reeds drawn beside them.
#[derive(Clone, Debug, Default)]
pub struct Streams {
  /// Streamed trees, when there are procedural trees.
  pub trees: Option<Stream>,
  /// Streamed grass, when grass is on.
  pub grass: Option<Stream>,
  /// Streamed boulders, when boulders are on and the ground has talus.
  pub boulders: Option<Stream>,
  /// Reeds, placed on the CPU.
  pub reeds: u32,
  /// Trees in the far set: every tree when nothing is streamed.
  pub far_trees: u32,
  /// The tree triangle budget.
  pub triangles: TriangleBudget,
  /// Whether the tree, grass and boulder generators cannot run yet (see
  /// `Gpu::generators_ready`): no tile of that kind is handed out, so
  /// none is recorded as filled while nothing fills it.
  pub waiting: [bool; 3],
  tree_radius: Option<DetailRadius>,
  grass_radius: Option<DetailRadius>,
  boulder_radius: Option<DetailRadius>,
}

/// What the trees drew in one frame, read back from the GPU.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TreeDraw {
  /// Trees drawn as full meshes.
  pub meshes: u32,
  /// Their triangles, from each species' mesh.
  pub mesh_triangles: u64,
  /// Impostor triangles: two a tree, four for crossed quads.
  pub impostor_triangles: u64,
  /// Trees drawn into the shadow map, one quad each.
  pub shadow_casters: u32,
}

impl TreeDraw {
  /// Every tree triangle drawn: meshes, impostors and shadow casters.
  pub fn triangles(&self) -> u64 {
    self.mesh_triangles + self.impostor_triangles + u64::from(self.shadow_casters) * 2
  }
}

/// Share of the tree triangle budget the shadow casters may use.
pub const SHADOW_SHARE: f64 = 0.25;
/// Frames a new distance is left to show in the read-back counts, which
/// arrive a frame or two late, before it is judged again.
const SETTLE_FRAMES: u32 = 3;
/// Seconds with room to spare before a distance grows back a step.
const TRIANGLE_REGROW_SECONDS: f32 = 1.0;

/// Holds the tree triangles drawn to their budget, from the counts the
/// GPU reads back. Over budget, the mesh distance comes in, so the
/// furthest meshes become impostors first and no tree is dropped; the
/// shadow casters' reach from the camera comes in while they fill their
/// share. Both grow back a step at a time once there is room.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TriangleBudget {
  /// The mesh distance the budget allows, when it limits it.
  mesh_reach: Option<f32>,
  /// The shadow casters' reach, when the budget limits it.
  shadow_reach: Option<f32>,
  /// The distances last applied.
  applied: [f32; 2],
  settle: u32,
  calm: f32,
  shadow_calm: f32,
}

impl TriangleBudget {
  /// The most shadow casters for a budget of `triangles`.
  pub fn max_shadow_casters(triangles: u32) -> u32 {
    (f64::from(triangles) * SHADOW_SHARE / 2.0).min(f64::from(u32::MAX)) as u32
  }

  /// Judge one frame's `draw` against `budget` triangles, `dt` seconds
  /// after the last. `most` is the flora's mesh distance, and `shadow`
  /// the tree shadow distance.
  pub fn observe(&mut self, draw: &TreeDraw, budget: u32, most: f32, shadow: f32, dt: f32) {
    if self.settle > 0 {
      self.settle -= 1;
      return;
    }

    let budget = f64::from(budget);
    let shadows = u64::from(draw.shadow_casters) * 2;
    let meshes_allowed = (budget - (draw.impostor_triangles + shadows) as f64).max(budget * 0.25);
    let mesh_triangles = draw.mesh_triangles as f64;
    let [mesh_applied, shadow_applied] = self.applied;

    if mesh_triangles > meshes_allowed {
      // Trees within a distance go with its square in a closed stand.
      let step = (meshes_allowed / mesh_triangles).sqrt().clamp(0.5, 0.95) as f32;
      self.mesh_reach = Some((mesh_applied * step).max(1.0));
      self.settle = SETTLE_FRAMES;
      self.calm = 0.0;
    } else if let Some(reach) = self
      .mesh_reach
      .filter(|_| mesh_triangles < meshes_allowed * 0.7)
    {
      self.calm += dt;

      if self.calm >= TRIANGLE_REGROW_SECONDS {
        let grown = reach.max(mesh_applied) / 0.9;
        self.mesh_reach = (grown < most).then_some(grown);
        self.settle = SETTLE_FRAMES;
        self.calm = 0.0;
      }
    } else {
      self.calm = 0.0;
    }

    let casters = f64::from(draw.shadow_casters);
    let cap = f64::from(Self::max_shadow_casters(budget as u32));

    if casters >= cap {
      self.shadow_reach = Some((shadow_applied * 0.8).max(10.0));
      self.settle = SETTLE_FRAMES;
      self.shadow_calm = 0.0;
    } else if let Some(reach) = self.shadow_reach.filter(|_| casters < cap * 0.7) {
      self.shadow_calm += dt;

      if self.shadow_calm >= TRIANGLE_REGROW_SECONDS {
        let grown = reach.max(shadow_applied) / 0.9;
        self.shadow_reach = (grown < shadow).then_some(grown);
        self.settle = SETTLE_FRAMES;
        self.shadow_calm = 0.0;
      }
    } else {
      self.shadow_calm = 0.0;
    }
  }

  /// The mesh distance to use, at most `most`.
  pub fn mesh_metres(&self, most: f32) -> f32 {
    self.mesh_reach.map_or(most, |reach| reach.min(most))
  }

  /// The shadow casters' reach to use, at most `most`.
  pub fn shadow_metres(&self, most: f32) -> f32 {
    self.shadow_reach.map_or(most, |reach| reach.min(most))
  }

  /// Record the distances a frame used.
  pub fn applied(&mut self, mesh: f32, shadow: f32) {
    self.applied = [mesh, shadow];
  }
}

/// What streaming does in one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamFrame {
  /// Tree tiles to generate, and slots emptied.
  pub trees: crate::render::lattice::TileChanges,
  /// Grass tiles to generate, and slots emptied.
  pub grass: crate::render::lattice::TileChanges,
  /// Full-density radius for trees and grass, in metres.
  pub tree_radius: f32,
  /// See `tree_radius`.
  pub grass_radius: f32,
  /// Where tufts start to hand over to the ground's grass sheen, in
  /// metres (see [`crate::render::grass::handover_start`]).
  pub grass_handover: f32,
  /// Trees and tufts expected to be drawn.
  pub trees_drawn: u32,
  /// See `trees_drawn`; reeds included.
  pub grass_drawn: u32,
  /// How far trees may be drawn as full meshes, when streaming or the
  /// triangle budget caps it (see [`Stream::mesh_reach`] and
  /// [`TriangleBudget`]).
  pub mesh_metres: Option<f32>,
  /// How far from the camera trees cast shadows (see [`TriangleBudget`]).
  pub shadow_metres: f32,
  /// Most trees drawn into the shadow map.
  pub max_shadow_casters: u32,
  /// Boulder tiles to generate, and slots emptied.
  pub boulders: crate::render::lattice::TileChanges,
  /// How far boulders are drawn this frame, in metres: the boulder
  /// distance, less under detail pressure or over the budget.
  pub boulder_metres: f32,
  /// Boulders expected to be drawn.
  pub boulders_drawn: u32,
}

impl Default for StreamFrame {
  fn default() -> Self {
    Self {
      trees: Default::default(),
      grass: Default::default(),
      tree_radius: 0.0,
      grass_radius: 0.0,
      grass_handover: 0.0,
      trees_drawn: 0,
      grass_drawn: 0,
      mesh_metres: None,
      shadow_metres: f32::MAX,
      max_shadow_casters: u32::MAX,
      boulders: Default::default(),
      boulder_metres: 0.0,
      boulders_drawn: 0,
    }
  }
}

/// The grass's full-cover radius never shrinks below this, in metres
/// (or `grassDetailMetres`, when that is smaller).
const GRASS_FLOOR_METRES: f32 = 10.0;

/// Most trees drawn as full meshes: this share of `maxTreeInstances`.
/// A tree mesh is up to 4,000 triangles, an impostor two, so in a closed
/// canopy the meshes, not the count, bound the trees pass.
pub const MESH_SHARE: u32 = 20;

impl Streams {
  /// Bring the streams up to date for a camera at `camera` looking
  /// through `planes`: estimate what is drawn, let the budgets shrink or
  /// regrow each near radius, halve it at most under `pressure` (see
  /// `pacing::ResolutionController::detail_pressure`, and the boulders'
  /// reach from `boulder_distance` likewise, which is `None` while their
  /// pipelines are not wanted, so no tile is handed out that nothing would
  /// fill), bring the mesh
  /// distance (at most `mesh_distance`) in so no more than
  /// `max_trees / MESH_SHARE` trees are full meshes (fewer under
  /// pressure, with the area the radius keeps) and the triangle budget
  /// holds (see [`TriangleBudget`], with trees casting shadows at most
  /// `shadow_distance` from the camera), and choose the tiles to
  /// (re)generate, nearest first.
  #[allow(clippy::too_many_arguments)]
  pub fn update(
    &mut self,
    ground: &GroundData,
    camera: [f32; 3],
    planes: &[[f32; 4]; 6],
    budget: &vista_types::VegetationBudget,
    pressure: f32,
    dt: f32,
    grass_view: f32,
    mesh_distance: f32,
    shadow_distance: f32,
    boulder_distance: Option<f32>,
  ) -> StreamFrame {
    use crate::render::lattice::TILES_PER_FRAME;
    let mut frame = StreamFrame {
      max_shadow_casters: TriangleBudget::max_shadow_casters(budget.max_tree_triangles),
      ..StreamFrame::default()
    };
    let squeeze = 1.0 - 0.5 * pressure.clamp(0.0, 1.0);
    let at = [camera[0], camera[2]];
    let jobs = |waiting: bool, most: usize| if waiting { 0 } else { most };

    if let Some(stream) = &mut self.trees {
      let control = self
        .tree_radius
        .get_or_insert_with(|| DetailRadius::new(budget.detail_metres, 60.0));
      control.set_most(budget.detail_metres, 60.0);
      let radius = control.radius * squeeze;
      let reach = stream
        .layout
        .classes
        .last()
        .map_or(0.0, |class| class.reach);
      let drawn = stream.estimate(
        ground,
        camera,
        planes,
        radius,
        f32::MAX,
        budget.canopy_metres,
        false,
      );
      frame.tree_radius = control.update(drawn, budget.max_trees, dt) * squeeze;
      frame.trees_drawn = drawn;
      frame.mesh_metres = stream.mesh_reach(
        ground,
        camera,
        planes,
        frame.tree_radius,
        mesh_distance,
        (budget.max_trees as f32 / MESH_SHARE as f32 * squeeze * squeeze) as u32,
      );
      let mass = &stream.mass;
      frame.trees = stream.ring.update(
        &stream.layout,
        at,
        &|tx, tz| mass.wanted(tx, tz),
        &|distance| if distance <= reach { 1.0 } else { 0.0 },
        jobs(self.waiting[0], TILES_PER_FRAME),
      );
    } else {
      frame.trees_drawn = self.far_trees;
    }

    // The triangle budget brings the mesh distance in further, and the
    // shadow casters' reach.
    let meshes = self.triangles.mesh_metres(mesh_distance);
    let mesh = frame.mesh_metres.map_or(meshes, |reach| reach.min(meshes));
    frame.mesh_metres = (mesh < mesh_distance).then_some(mesh);
    frame.shadow_metres = self.triangles.shadow_metres(shadow_distance);
    self.triangles.applied(mesh, frame.shadow_metres);

    if let Some(stream) = &mut self.grass {
      let most = grass_radius(budget, grass_view).min(stream.radius);
      let control = self
        .grass_radius
        .get_or_insert_with(|| DetailRadius::new(most, GRASS_FLOOR_METRES));
      control.set_most(most, GRASS_FLOOR_METRES);
      let radius = control.radius * squeeze;
      let drawn = stream.estimate(ground, camera, planes, radius, grass_view, f32::MAX, true);
      frame.grass_radius = control.update(drawn + self.reeds, budget.max_grass, dt) * squeeze;
      frame.grass_drawn = drawn + self.reeds;
      let (mass, radius) = (&stream.mass, frame.grass_radius);
      frame.grass_handover = crate::render::grass::handover_start(radius, grass_view);
      frame.grass = stream.ring.update(
        &stream.layout,
        at,
        &|tx, tz| mass.wanted(tx, tz),
        &|distance| grass_keep(distance, radius, grass_view),
        jobs(self.waiting[1], TILES_PER_FRAME * 2),
      );
    } else {
      frame.grass_drawn = self.reeds;
    }

    if let (Some(stream), Some(boulder_distance)) = (&mut self.boulders, boulder_distance) {
      // Pressure and the budget bring the reach in, never below a sixth
      // of the boulder distance; the scree texture carries the look on.
      let floor = boulder_distance / 6.0;
      let control = self
        .boulder_radius
        .get_or_insert_with(|| DetailRadius::new(boulder_distance, floor));
      control.set_most(boulder_distance, floor);
      let reach = control.radius * squeeze;
      let drawn = stream.estimate(ground, camera, planes, reach, reach, f32::MAX, true);
      let reach = control.update(drawn, crate::render::boulders::MAX_BOULDERS, dt) * squeeze;
      frame.boulder_metres = reach;
      frame.boulders_drawn = drawn;
      let (mass, tile) = (&stream.mass, stream.layout.tile);
      frame.boulders = stream.ring.update(
        &stream.layout,
        at,
        &|tx, tz| {
          mass.wanted(tx, tz)
            && crate::render::lattice::tile_distance(tile, tx, tz, at[0], at[1]) <= reach
        },
        &|_| 1.0,
        jobs(self.waiting[2], TILES_PER_FRAME),
      );
    }

    frame
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// A tile grid over a vast map would take gigabytes; past `MAX_TILES`
  /// the mass is empty instead, so nothing is streamed.
  #[test]
  fn a_tile_grid_past_its_cap_is_empty_not_gigabytes() {
    let ground = GroundData {
      width: 2,
      height: 2,
      half: [200_000.0, 200_000.0],
      texel_metres: 400_000.0,
      ..GroundData::default()
    };
    assert!(!TileMass::fits(&ground, 16.0));
    let mass = TileMass::build(&ground, 16.0, 0.0, &|_| 1.0);
    assert_eq!(mass.most(), 0.0);
    assert_eq!(mass.tiles().count(), 0);
    assert!(TileMass::fits(&ground, 64.0));
  }
  use crate::render::lattice::{thinning, FAR_KEEP, TREE_TILE_METRES};
  use vista_types::{FloraOptions, TerrainMetadata};

  /// A level 1 km square at 40 m, 4 m samples, whose cover texture says
  /// `share` of the table density everywhere, with forest species.
  fn level_ground(red: u8) -> (HeightMap, GroundData) {
    let size = 251;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 4.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let map = HeightMap::flat(size, size, 40.0, metadata);
    let mut ground = GroundData::of(&map);
    let surface = vec![SurfaceSample::default(); map.heights.len()];
    ground.set_surface(&map, &surface, &Default::default(), &[]);
    ground.cover = vec![[red, TreeSpecies::Oak as u8, TreeSpecies::Pine as u8, 64]; surface.len()];
    (map, ground)
  }

  #[test]
  fn no_tile_is_handed_out_while_its_generator_waits() {
    let (map, mut ground) = level_ground(0);
    let surface = vec![
      SurfaceSample {
        materials: [255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        celsius_hundredths: 1200,
        ..SurfaceSample::default()
      };
      map.heights.len()
    ];
    ground.set_surface(&map, &surface, &Default::default(), &[]);
    ground.cover = vec![[0; 4]; surface.len()];
    ground.grass = crate::render::grass::bake_grass(&map, &surface, &ground);
    let rules =
      crate::render::grass::GrassRules::new(&vista_types::GrassOptions::default(), 0.5, 0.0);
    let mut streams = Streams {
      grass: Some(grass_stream(
        TileMass::grass(&ground, &rules, &[]),
        45.0,
        220.0,
        200_000,
      )),
      waiting: [true; 3],
      ..Streams::default()
    };
    let everywhere = [[0.0, 0.0, 0.0, 1.0]; 6];
    let budget = vista_types::RenderQualityOptions::default().vegetation();
    let update = |streams: &mut Streams| {
      streams.update(
        &ground,
        [10.0, 42.0, 10.0],
        &everywhere,
        &budget,
        0.0,
        1.0 / 60.0,
        220.0,
        420.0,
        200.0,
        None,
      )
    };

    // While the pipelines build, nothing is handed out, however many
    // frames pass.
    for _ in 0..5 {
      assert!(update(&mut streams).grass.jobs.is_empty());
    }

    // Then the tiles round the camera come first.
    streams.waiting = [false; 3];
    let first = update(&mut streams).grass;
    assert_eq!(
      first.jobs.len(),
      crate::render::lattice::TILES_PER_FRAME * 2
    );
    assert!(first.jobs.iter().any(|job| job.tile == [0, 0]));
  }

  #[test]
  fn the_far_set_and_the_tiles_place_each_point_once() {
    let (map, ground) = level_ground(80);
    let bins = channel_bins(&map, None);
    let rules = TreeRules::new(&FloraOptions::default(), 4.0);
    let all = lattice_trees(&ground, &bins, &rules, 1.0, usize::MAX);
    let far = lattice_trees(&ground, &bins, &rules, FAR_KEEP, usize::MAX);
    let key = |tree: &TreeInstance| (tree.position[0].to_bits(), tree.position[2].to_bits());
    let mut seen = std::collections::HashSet::new();

    for tree in &far {
      assert!(seen.insert(key(tree)));
    }

    let tiles = (256.0 / TREE_TILE_METRES) as i32;

    for tz in -tiles..tiles {
      for tx in -tiles..tiles {
        for tree in tile_trees(&ground, &bins, &rules, [tx, tz], 1.0) {
          assert!(seen.insert(key(&tree)), "{:?} placed twice", tree.position);
        }
      }
    }

    // Together they are every tree of the square, and nothing else.
    let expected: std::collections::HashSet<_> = all.iter().map(key).collect();
    assert!(seen.is_subset(&expected));

    for tree in &all {
      if tree.position[0].abs() < 250.0 && tree.position[2].abs() < 250.0 {
        assert!(seen.contains(&key(tree)), "{:?} is missing", tree.position);
      }
    }

    assert!(far.len() * 5 < all.len() && far.len() * 12 > all.len());
  }

  #[test]
  fn a_region_holds_the_far_set_and_the_streamed_tiles_there() {
    let (map, ground) = level_ground(80);
    let bins = channel_bins(&map, None);
    let rules = TreeRules::new(&FloraOptions::default(), 4.0);
    let region = [-100.0, -60.0, 70.0, 90.0];
    let inside = |tree: &TreeInstance| {
      let [x, _, z] = tree.position;
      x >= region[0] && x <= region[2] && z >= region[1] && z <= region[3]
    };
    let key = |tree: &TreeInstance| {
      (
        tree.position[0].to_bits(),
        tree.position[2].to_bits(),
        tree.species,
        tree.scale.to_bits(),
      )
    };
    // As the renderer holds them: the far set, and every streamed tile's
    // trees, each tile filled as `tree_generate.wgsl` fills it.
    let mut drawn: Vec<_> = lattice_trees(&ground, &bins, &rules, FAR_KEEP, usize::MAX)
      .iter()
      .filter(|tree| inside(tree))
      .map(key)
      .collect();

    for tz in -3..3 {
      for tx in -3..3 {
        drawn.extend(
          tile_trees(&ground, &bins, &rules, [tx, tz], 1.0)
            .iter()
            .filter(|tree| inside(tree))
            .map(key),
        );
      }
    }

    let exported = region_trees(&ground, &bins, &rules, region, usize::MAX).unwrap();
    let mut keys: Vec<_> = exported.iter().map(key).collect();
    drawn.sort_unstable();
    keys.sort_unstable();
    assert!(keys.len() > 300, "{} trees", keys.len());
    assert_eq!(keys, drawn);

    // The same again, and an error one tree short.
    assert_eq!(
      region_trees(&ground, &bins, &rules, region, usize::MAX).unwrap(),
      exported
    );
    assert_eq!(
      region_trees(&ground, &bins, &rules, region, exported.len() - 1),
      Err(exported.len() - 1)
    );
    assert_eq!(
      region_trees(&ground, &bins, &rules, region, exported.len()).map(|trees| trees.len()),
      Ok(exported.len())
    );
  }

  #[test]
  fn the_triangle_budget_turns_the_furthest_meshes_into_impostors() {
    // A dense stand: a tree every 3 m out to 400 m, every tree a
    // 3,000-triangle mesh within the mesh distance, as the cull pass
    // decides, and a two-triangle impostor beyond it.
    let distances: Vec<f32> = (-133..=133)
      .flat_map(|z| (-133..=133).map(move |x| crate::maths::length2(x as f32, z as f32) * 3.0))
      .filter(|distance| *distance <= 400.0)
      .collect();
    let (budget, most, shadow) = (2_500_000u32, 420.0f32, 300.0f32);
    let draw_at = |mesh: f32, casters: f32| {
      let meshes = distances.iter().filter(|d| **d < mesh).count() as u32;
      TreeDraw {
        meshes,
        mesh_triangles: u64::from(meshes) * 3_000,
        impostor_triangles: (distances.len() as u64 - u64::from(meshes)) * 2,
        shadow_casters: distances.iter().filter(|d| **d < casters).count() as u32,
      }
    };
    let mut control = TriangleBudget::default();
    let (mut mesh, mut casters) = (most, shadow);

    for _ in 0..200 {
      control.applied(mesh, casters);
      let draw = draw_at(mesh, casters);
      // The GPU caps the casters at their share.
      let draw = TreeDraw {
        shadow_casters: draw
          .shadow_casters
          .min(TriangleBudget::max_shadow_casters(budget)),
        ..draw
      };
      control.observe(&draw, budget, most, shadow, 1.0 / 60.0);
      mesh = control.mesh_metres(most);
      casters = control.shadow_metres(shadow);
    }

    let draw = draw_at(mesh, casters);
    assert!(draw.triangles() <= u64::from(budget), "{draw:?}");
    // Most of the budget is used: the distance is not cut far short.
    assert!(draw.triangles() > u64::from(budget) / 2, "{draw:?}");
    // Every tree is still drawn, as a mesh or an impostor, and every mesh
    // is nearer than every impostor.
    assert_eq!(
      draw.meshes as usize + (draw.impostor_triangles / 2) as usize,
      distances.len()
    );
    let nearest_impostor = distances
      .iter()
      .filter(|d| **d >= mesh)
      .fold(f32::MAX, |a, d| a.min(*d));
    let furthest_mesh = distances
      .iter()
      .filter(|d| **d < mesh)
      .fold(0.0f32, |a, d| a.max(*d));
    assert!(furthest_mesh < nearest_impostor);
    assert!(mesh < most && draw.meshes > 0);
    // The shadow casters stay within their quarter of the budget.
    assert!(u64::from(draw.shadow_casters) * 2 <= u64::from(budget) / 4);

    // A light scene grows back to the full mesh distance.
    let light = TreeDraw {
      meshes: 10,
      mesh_triangles: 30_000,
      impostor_triangles: 1_000,
      shadow_casters: 100,
    };

    for _ in 0..2_000 {
      control.applied(control.mesh_metres(most), control.shadow_metres(shadow));
      control.observe(&light, budget, most, shadow, 1.0 / 60.0);
    }

    assert_eq!(control.mesh_metres(most), most);
    assert_eq!(control.shadow_metres(shadow), shadow);
  }

  #[test]
  fn the_mesh_distance_holds_the_meshes_to_their_budget() {
    // A 1 km square of dense forest, the camera over its centre, and a
    // frustum that sees everything.
    let (_, ground) = level_ground(255);
    let rules = TreeRules::new(&FloraOptions::default(), 4.0);
    let stream = tree_stream(
      TileMass::trees(&ground, &rules, &[]),
      250.0,
      u32::MAX,
      FAR_KEEP,
    );
    let planes = [[0.0, 0.0, 0.0, 1.0]; 6];
    let camera = [0.0, 60.0, 0.0];
    let reach = |meshes: u32| stream.mesh_reach(&ground, camera, &planes, 250.0, 420.0, meshes);
    let (few, more) = (reach(2_000).unwrap(), reach(6_000).unwrap());
    assert!(few < more && more < 420.0, "{few} m, {more} m");
    // The trees within the reach are about the budget.
    let density = stream.mass.at(0, 0) / (64.0 * 64.0);
    let within = density * std::f32::consts::PI * more * more;
    assert!(
      (within / 6_000.0 - 1.0).abs() < 0.35,
      "{within} trees within {more} m"
    );
    // A budget the stand cannot fill leaves the mesh distance alone.
    assert_eq!(reach(10_000_000), None);
  }

  #[test]
  fn dense_stands_grow_an_understorey() {
    let (map, ground) = level_ground(80);
    let bins = channel_bins(&map, None);
    let small = |density: f32| {
      let rules = TreeRules::new(&FloraOptions::default(), density);
      let trees = lattice_trees(&ground, &bins, &rules, 1.0, usize::MAX);
      let under = trees.iter().filter(|tree| tree.scale <= 0.5).count();
      (under as f32 / trees.len() as f32, trees.len())
    };
    // Up to the old maximum, no tree is understorey (young trees are
    // 0.45 and up, and only a few reach 0.5).
    let (one, _) = small(1.0);
    assert!(one < 0.15, "{one}");
    // At 4 the canopy trees stay about as many as at 1; the rest are
    // saplings beneath them.
    let (four, count) = small(4.0);
    let canopy = (1.0 - four) * count as f32;
    let (_, count_one) = small(1.0);
    assert!(four > 0.85, "{four}");
    assert!(
      canopy < 1.5 * count_one as f32,
      "{canopy} canopy trees, {count_one} at 1"
    );
  }

  #[test]
  fn thinning_keeps_the_canopy_closed_in_every_annulus() {
    // Crown area kept in annuli round the camera, over the lattice points
    // themselves, against every tree drawn at full density.
    let radius = 250.0;
    let p = 0.6;
    let seed = 91;

    for centre in [100.0f32, 300.0, 800.0, 1600.0] {
      let (inner, outer) = (centre - 40.0, centre + 40.0);
      let reach = (outer / TREE_PITCH).ceil() as i32;
      let (mut full, mut kept) = (0.0f64, 0.0f64);

      for iz in -reach..=reach {
        for ix in -reach..=reach {
          let hash = point_hash(ix, iz, seed);
          let [x, z] = jittered(ix, iz, hash, TREE_PITCH);
          let distance = crate::maths::length2(x, z);
          let rank = unit(hash[2]);

          if distance < inner || distance >= outer || rank >= p {
            continue;
          }

          let t = thinning(distance, radius, FAR_KEEP);
          let size = t.fade(rank / p) * t.height * t.width;
          full += 1.0;
          kept += f64::from(size * size);
        }
      }

      assert!(
        (kept / full - 1.0).abs() < 0.07,
        "{centre} m: {:.3} of the crown area",
        kept / full
      );
    }
  }

  /// A brook 2 m wide looping across a plain on a 12 m grid.
  fn looping_brook() -> Vec<[f32; 3]> {
    (0..1400)
      .map(|i| {
        let x = -700.0 + i as f32;
        let z = 30.0 + 5.0 * (x / 22.0 * std::f32::consts::TAU).sin() + 3.0 * (x / 9.0).cos();
        [x, z, 1.0]
      })
      .collect()
  }

  #[test]
  fn trees_take_variants_ages_and_leans_from_their_ground() {
    let (map, mut ground) = level_ground(200);
    // A straight brook along z = 30.
    let rivers = crate::render::water::RiverNetwork {
      channels: vec![(0..40)
        .map(|i| [-400.0 + i as f32 * 20.0, 30.0, 1.0])
        .collect()],
      falls: Vec::new(),
      ..Default::default()
    };
    let bins = channel_bins(&map, Some(&rivers));
    let options = FloraOptions::default();
    let rules = TreeRules::new(&options, 1.0);
    let trees = lattice_trees(&ground, &bins, &rules, 1.0, usize::MAX);
    let mut variants = [0; 4];
    let mut ages = [0; 4];

    for tree in &trees {
      variants[tree.variant() as usize] += 1;
      ages[tree.age() as usize] += 1;
      let (sector, strength) = tree.lean();
      // The brook runs from x = -400 to 380: beyond its ends the water is
      // its end point.
      let along = tree.position[0].clamp(-400.0, 380.0);
      let off_water = (tree.position[0] - along).hypot(tree.position[2] - 30.0);

      if off_water < LEAN_WATER_METRES - 0.5 && along == tree.position[0] {
        // Bank trees lean over the water, and are never old.
        let towards = if tree.position[2] < 30.0 { 0 } else { 8 };
        assert_eq!(sector, towards, "at {:?}", tree.position);
        assert!(strength >= 2);
        assert_ne!(tree.age(), Age::Old as u32);
      } else if off_water > LEAN_WATER_METRES + 1.0 {
        // Level ground: with the prevailing wind, at most 4 degrees.
        assert_eq!(sector, 2, "at {:?}", tree.position);
        assert!(strength <= 2);
      }
    }

    let total = trees.len() as f32;
    assert!(total > 1000.0);

    for count in variants {
      assert!((count as f32 / total - 0.25).abs() < 0.03, "{variants:?}");
    }

    // Seedlings are young, and some of the rest are old; nothing is
    // stunted here.
    assert!(ages[Age::Young as usize] as f32 > total * 0.25, "{ages:?}");
    assert!(ages[Age::Old as usize] as f32 > total * 0.1, "{ages:?}");
    assert_eq!(ages[Age::Krummholz as usize], 0);

    // Stunted trees grow as krummholz, turned into the prevailing wind.
    for texel in &mut ground.cover {
      texel[3] |= crate::render::flora::COVER_STUNTED;
    }

    let stunted = lattice_trees(&ground, &bins, &rules, 1.0, usize::MAX);
    let windward = PREVAILING_WIND[1].atan2(-PREVAILING_WIND[0]);
    assert!(!stunted.is_empty());

    for tree in &stunted {
      assert!(tree.stunted());
      assert_eq!(tree.age(), Age::Krummholz as u32);
      assert!(
        (tree.rotation - windward).abs() <= 0.36,
        "{}",
        tree.rotation
      );
    }
  }

  #[test]
  fn the_channel_bins_exclude_exactly_what_placement_excludes() {
    let metadata = TerrainMetadata {
      width: 128,
      height: 128,
      metres_per_sample: 12.0,
      ..TerrainMetadata::default()
    };
    let map = HeightMap::flat(128, 128, 40.0, metadata);
    let rivers = crate::render::water::RiverNetwork {
      channels: vec![looping_brook()],
      falls: Vec::new(),
      ..Default::default()
    };
    let half = [127.0 * 6.0, 127.0 * 6.0];
    let waters = crate::render::flora::waters(&map, &rivers, half);
    let bins = channel_bins(&map, Some(&rivers));
    let (mut inside, mut checked) = (0, 0);

    for iz in -500..500 {
      for ix in -300..300 {
        let (x, z) = (ix as f32 * 0.37 + 0.11, 30.0 + iz as f32 * 0.043);
        let mut cpu = (false, false);
        waters.for_each_near(x, z, |water| {
          cpu.0 |= water.intrusion(x, z) > 0.0;
          cpu.1 |= water.intrusion(x, z) - water.clearance > 0.0;
        });
        assert_eq!(in_channel(&bins, x, z, true), cpu.0, "trees at ({x}, {z})");
        assert_eq!(in_channel(&bins, x, z, false), cpu.1, "grass at ({x}, {z})");
        inside += usize::from(cpu.0);
        checked += 1;
      }
    }

    assert!(
      inside > checked / 20 && inside < checked / 2,
      "{inside} of {checked}"
    );
    // No channels: nothing is excluded.
    assert!(!in_channel(&channel_bins(&map, None), 0.0, 30.0, true));
  }

  #[test]
  fn tiles_partition_the_lattice() {
    // Every tree lattice index belongs to exactly one tile, in metres,
    // and every grass index likewise, in centimetres.
    for (size, pitch) in [(64, 3), (1600, 35)] {
      for tile in -40..40 {
        let (first, next) = (
          tile_first(tile, size, pitch),
          tile_first(tile + 1, size, pitch),
        );
        assert!(first * pitch >= tile * size && (first - 1) * pitch < tile * size);
        assert!(next > first);
      }
    }
  }

  /// A level square with a straight stream 4 m wide along z = 0 whose
  /// band has density `band`, in `biome` at 12 °C (with `snow` lying all
  /// year), and no cover: only riparian scrub grows.
  fn riparian_ground(biome: BiomeKind, snow: u8, band: f32) -> (GroundData, Vec<u32>, TreeRules) {
    let (map, mut ground) = level_ground(0);
    let surface = vec![
      SurfaceSample {
        biome: biome as u8,
        permanent_snow: snow,
        celsius_hundredths: 1200,
        ..SurfaceSample::default()
      };
      map.heights.len()
    ];
    ground.set_surface(&map, &surface, &Default::default(), &[]);
    ground.cover = vec![[0; 4]; surface.len()];
    let run: Vec<[f32; 3]> = (0..=200)
      .map(|i| [-400.0 + 4.0 * i as f32, 0.0, 2.0])
      .collect();
    let rivers = crate::render::water::RiverNetwork {
      bands: vec![vec![band; run.len()]],
      channels: vec![run],
      ..Default::default()
    };
    let bins = channel_bins(&map, Some(&rivers));
    (ground, bins, TreeRules::new(&FloraOptions::default(), 1.0))
  }

  #[test]
  fn riparian_scrub_lines_streams_from_one_metre_to_its_band() {
    let (ground, bins, rules) = riparian_ground(BiomeKind::GrassyMeadows, 0, 1.0);
    let scrub = lattice_trees(&ground, &bins, &rules, 1.0, usize::MAX);
    // About 0.45 of the 3 m points over 2 x 800 m x 5 m, less the gaps.
    assert!((80..400).contains(&scrub.len()), "{} scrub", scrub.len());
    let mut sides = [Vec::new(), Vec::new()];

    for tree in &scrub {
      let [x, _, z] = tree.position;
      let edge = crate::maths::length2((x.abs() - 400.0).max(0.0), z) - 2.0;
      assert!(
        (1.0..=6.0 + 1e-4).contains(&edge),
        "scrub {edge} m from the water"
      );
      let species = tree.species & 0xff;
      assert!(species == TreeSpecies::Shrub as u32 || species == TreeSpecies::Oak as u32);
      // 2 to 6 m tall, and leaning over the water.
      let height =
        crate::render::tree_growth::species_height(TreeSpecies::ALL[species as usize]) * tree.scale;
      assert!((1.5..6.5).contains(&height), "scrub {height} m tall");
      assert!(tree.lean().1 >= 2);
      sides[usize::from(z > 0.0)].push(x);
    }

    // Clumps, with gaps of 10 m or more where the water shows through.
    for side in &mut sides {
      side.sort_by(f32::total_cmp);
      let widest = side
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .fold(0.0, f32::max);
      assert!(widest >= 10.0, "no gap in the scrub: {widest} m at most");
    }

    // Twice the density gives about twice the scrub; none without it.
    let (ground, bins, rules) = riparian_ground(BiomeKind::GrassyMeadows, 0, 2.0);
    let dense = lattice_trees(&ground, &bins, &rules, 1.0, usize::MAX).len();
    assert!(
      dense as f32 > 1.6 * scrub.len() as f32,
      "{dense} against {}",
      scrub.len()
    );
    let (ground, bins, rules) = riparian_ground(BiomeKind::GrassyMeadows, 0, 0.0);
    assert!(lattice_trees(&ground, &bins, &rules, 1.0, usize::MAX).is_empty());
  }

  #[test]
  fn no_riparian_scrub_above_the_trees_on_snow_or_far_from_desert_water() {
    for biome in [
      BiomeKind::AlpineTransition,
      BiomeKind::IceArctic,
      BiomeKind::UpperSnowyPeaks,
    ] {
      let (ground, bins, rules) = riparian_ground(biome, 0, 1.0);
      assert!(
        lattice_trees(&ground, &bins, &rules, 1.0, usize::MAX).is_empty(),
        "{biome:?}"
      );
    }

    let (ground, bins, rules) = riparian_ground(BiomeKind::GrassyMeadows, 200, 1.0);
    assert!(lattice_trees(&ground, &bins, &rules, 1.0, usize::MAX).is_empty());

    // A desert's line of green is narrow.
    let (ground, bins, rules) = riparian_ground(BiomeKind::SavannahExpanse, 0, 1.0);
    let scrub = lattice_trees(&ground, &bins, &rules, 1.0, usize::MAX);
    assert!(!scrub.is_empty());
    assert!(scrub
      .iter()
      .all(|tree| tree.position[2].abs() - 2.0 <= SCRUB_DESERT));
  }

  #[test]
  fn the_far_set_and_the_streamed_tiles_hold_each_scrub_once() {
    let (ground, bins, mut rules) = riparian_ground(BiomeKind::OuterForest, 0, 1.0);
    rules.far_keep = 0.3;
    let key = |tree: &TreeInstance| {
      (
        tree.position[0].to_bits(),
        tree.position[2].to_bits(),
        tree.species,
      )
    };
    let mut every: Vec<_> = lattice_trees(&ground, &bins, &rules, 1.0, usize::MAX)
      .iter()
      .map(key)
      .collect();
    let far = lattice_trees(&ground, &bins, &rules, rules.far_keep, usize::MAX);
    assert!(!far.is_empty() && far.len() < every.len());
    let mut split: Vec<_> = far.iter().map(key).collect();

    for tz in -8..8 {
      for tx in -8..8 {
        split.extend(
          tile_trees(&ground, &bins, &rules, [tx, tz], 1.0)
            .iter()
            .map(key),
        );
      }
    }

    every.sort_unstable();
    split.sort_unstable();
    assert_eq!(every, split);

    // A region holds the scrub standing in it.
    let region = region_trees(
      &ground,
      &bins,
      &rules,
      [-100.0, -20.0, 100.0, 20.0],
      usize::MAX,
    )
    .expect("under the limit");
    let inside = every
      .iter()
      .filter(|(x, z, _)| f32::from_bits(*x).abs() <= 100.0 && f32::from_bits(*z).abs() <= 20.0)
      .count();
    assert_eq!(region.len(), inside);

    // And the tiles' mass expects about as much scrub as grows.
    let rivers_mass = scrub_mass(
      &ground,
      &[(0..=200)
        .map(|i| [-400.0 + 4.0 * i as f32, 0.0, 2.0])
        .collect()],
      &[vec![1.0; 201]],
    );
    let expected: f32 = rivers_mass.iter().sum();
    let grown = every.len() as f32;
    assert!(
      expected > 0.5 * grown && expected < 2.0 * grown,
      "{expected} against {grown}"
    );
  }

  #[test]
  fn the_generator_places_scrub_by_the_same_rules() {
    let wgsl = include_str!("../shaders/tree_generate.wgsl");
    let table = SCRUB_SPECIES.map(|word| format!("{word}u")).join(", ");

    for line in [
      format!("const SCRUB_SPECIES = array<u32, 19>({table});"),
      format!("const RIPARIAN_SALT: u32 = {RIPARIAN_SALT:#x}u;"),
      format!("const SCRUB_CHANCE: f32 = {SCRUB_CHANCE};"),
      format!("const SCRUB_FROM: f32 = {SCRUB_FROM:.1};"),
      format!("const SCRUB_DESERT: f32 = {SCRUB_DESERT};"),
      format!(
        "const SAVANNAH: u32 = {}u;",
        BiomeKind::SavannahExpanse as u32
      ),
      "saturate((clump(cell, seed) - 0.15) / 0.5)".to_string(),
      "saturate((45.0 - slope_degrees(m, xz)) / 10.0)".to_string(),
      "surface.r < 97u".to_string(),
      "select(0.15 + 0.15 * size_roll, 1.0 + 1.3 * size_roll, shrub)".to_string(),
      "0.35 + 0.3 * unit(traits.z)".to_string(),
      "shape_bits(m, shape, xz, !shrub, false)".to_string(),
    ] {
      assert!(
        wgsl.contains(&line),
        "the tree generator no longer has {line}"
      );
    }
  }
}
