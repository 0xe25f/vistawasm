//! Soil depth: where bedrock shows through, and where scree lies below it.
//!
//! Rock is not simply steep ground. It shows where the soil is thin:
//! on convex shoulders and ridge crests, on steep slopes that shed their
//! soil, in the frost-shattered ground above the trees, and where a
//! harder layer of the bedrock crosses a slope and stands out as a crag
//! or ledge. Valley floors and footslopes, where water and soil gather,
//! hold the deepest soil. [`soil_field`] estimates the depth once per
//! terrain bake, at heightmap resolution:
//!
//! ```text
//! depth = base - slope_loss - convexity_loss + accumulation - frost - strata
//! ```
//!
//! clamped to 0 to 3 m, and rock shows as the depth falls from 0.6 m to
//! 0.15 m, with a three-sample noise that makes outcrop edges ragged and
//! lobed. Below each outcrop, fallen rock gathers as scree for 2 to 8
//! samples down the steepest descent while the slope stays over 25
//! degrees; where it eases, the talus cone ends. The talus field (scree,
//! and the ground just below each outcrop) tells the boulder generator
//! where fallen blocks lie and how far down their cone they came.

use vista_types::{LandformKind, Vec3};

use crate::maths::{hash_u64, smoothstep, value_noise};
use crate::terrain::drainage::DrainageArea;
use crate::terrain::heightmap::HeightMap;

/// Depth (metres) above which soil fully hides the rock.
pub const SOIL_COVERS_METRES: f32 = 0.6;
/// Depth (metres) below which rock is fully exposed.
pub const SOIL_BARE_METRES: f32 = 0.15;
/// Deepest soil the model holds, in metres.
pub const MAX_SOIL_METRES: f32 = 3.0;
/// Scree only lies on slopes steeper than this, in degrees: where the
/// slope eases below it, the talus cone ends.
pub const SCREE_SLOPE_DEGREES: f32 = 25.0;
/// Slopes, in degrees, over which steep ground sheds its soil: none below
/// the first, 2.2 m at the second. Slopes are measured across a sample
/// (10 m or so), which averages out the steepest faces, so ground that
/// reads as 40 degrees still carries forest in real mountains.
pub const SLOPE_LOSS_DEGREES: (f32, f32) = (34.0, 58.0);
/// The convexity index (height above the surrounding ring per metre out)
/// at which a crest has lost all the soil convexity takes, and a hollow
/// gained all it gathers: a crest between slopes of about 30 degrees.
pub const CONVEXITY_SCALE: f32 = 0.6;
/// Scree's angle of repose, in degrees: above it, scree slides on and
/// little stays.
pub const SCREE_REPOSE_DEGREES: f32 = 36.0;
/// Rock exposure from which a sample sheds scree.
pub const OUTCROP_EXPOSURE: f32 = 0.5;
/// Rock exposure from which the ground just below counts as an outcrop's
/// foot, where fallen blocks gather.
pub const FOOT_EXPOSURE: f32 = 0.6;
/// Boulders rest on ground up to about this steep, in degrees; steeper,
/// they roll on.
pub const BOULDER_REST_DEGREES: f32 = 40.0;
/// How far below an outcrop its foot reaches, in metres.
pub const FOOT_METRES: f32 = 12.0;
/// Most boulder probability the talus field encodes (its top four bits
/// run from 0 to this).
pub const MAX_TALUS_PROBABILITY: f32 = 0.6;

/// Layered bedrock: harder beds that stand out where they cross a slope.
/// A bed's phase is `(height + dip . (x, z) + warp) / period`, so beds
/// follow the contours on level ground and tilt gently across the map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strata {
  /// Vertical spacing of the beds, in metres.
  pub period: f32,
  /// Rise of a bed per metre along x and z: the tangent of its dip in
  /// the dip direction.
  pub dip: [f32; 2],
  /// Seed for the warp noise.
  pub seed: u64,
  /// How much of each hard bed crops out, from -0.5 (little) to 0.5
  /// (nearly all of it): see [`Strata::cropping`].
  pub outcrop: f32,
}

impl Strata {
  /// Beds for a landform: tight, nearly flat beds for `mesaDesert`, which
  /// read as its terraces and crop out almost everywhere; loose, steeper
  /// ones for `alpine` and `fjords`, whose crystalline cores are bedded
  /// only here and there; moderate ones elsewhere. The dip direction and
  /// the exact spacing come from `seed`.
  pub fn new(landform: LandformKind, seed: u64) -> Self {
    let (period, dip, outcrop) = match landform {
      LandformKind::MesaDesert => ((25.0, 32.0), (2.0, 3.0), 0.5),
      LandformKind::Alpine | LandformKind::Fjords => ((45.0, 60.0), (6.0, 8.0), -0.1),
      _ => ((30.0, 45.0), (3.0, 6.0), 0.25),
    };
    let roll = |salt: u64| (hash_u64(seed ^ salt) >> 40) as f32 / 16_777_216.0;
    let angle = roll(0x51a7) * std::f32::consts::TAU;
    let degrees = dip.0 + (dip.1 - dip.0) * roll(0xd1b5);
    let rise = degrees.to_radians().tan();
    Self {
      period: period.0 + (period.1 - period.0) * roll(0x9e3f),
      dip: [angle.cos() * rise, angle.sin() * rise],
      seed,
      outcrop,
    }
  }

  /// The bed phase, in periods, at world `(x, z)` and height `h` metres.
  /// The beds bend by up to a third of a period over a 400 m, two-octave
  /// noise, so they are never ruler-straight.
  pub fn phase(&self, x: f32, z: f32, h: f32) -> f32 {
    self.phase_warped(x, z, h, self.warp(x, z))
  }

  /// The bend in the beds at world `(x, z)`, in periods.
  pub fn warp(&self, x: f32, z: f32) -> f32 {
    let (u, v) = (x / 400.0, z / 400.0);
    (value_noise(self.seed ^ 0x57a7, u, v) * 0.67
      + value_noise(self.seed ^ 0x57a8, u * 2.03, v * 2.03) * 0.33)
      * 0.33
  }

  /// [`Strata::phase`] with the warp there already known.
  pub fn phase_warped(&self, x: f32, z: f32, h: f32, warp: f32) -> f32 {
    (h + self.dip[0] * x + self.dip[1] * z) / self.period + warp
  }

  /// How hard the bed at a phase is, 0 to 1: about a third of each period
  /// is a hard bed, with soft edges.
  pub fn hardness(phase: f32) -> f32 {
    smoothstep(((phase * std::f32::consts::TAU).sin() - 0.25) / 0.45)
  }

  /// How much of a hard bed crops out at world `(x, z)`, 0 to 1. Beds pinch
  /// out and hide under their own debris along the strike, so a crag runs
  /// for a hundred metres or two and gives way to turf before the next.
  pub fn cropping(&self, x: f32, z: f32) -> f32 {
    smoothstep((value_noise(self.seed ^ 0xc409, x / 120.0, z / 120.0) + self.outcrop) / 0.5)
  }
}

impl Default for Strata {
  fn default() -> Self {
    Self::new(LandformKind::Continental, 0)
  }
}

/// What the soil model needs beyond the heights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoilOptions {
  /// How readily bedrock shows through thin soil: 0 is deep soil
  /// everywhere, 1 the natural depth and 2 rocky (the soil half as deep).
  pub rockiness: f32,
  /// The bedrock's beds.
  pub strata: Strata,
}

impl Default for SoilOptions {
  fn default() -> Self {
    Self {
      rockiness: 1.0,
      strata: Strata::default(),
    }
  }
}

/// Climate at one sample, from the biome classifier.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Site {
  /// Moisture, 0 (arid) to 1 (wet).
  pub moisture: f32,
  /// How far into the frost-shattered ground above the trees, 0 (below
  /// the tree line) to 1 (at the snow line and above).
  pub frost: f32,
}

/// The climate the soil model reads, from the biome classifier. It varies
/// over kilometres, so the model reads it on a lattice about 32 m apart
/// and interpolates; only frost, which rises with each sample's own
/// height, is worked out per sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Climate {
  /// Moisture, 0 (arid) to 1 (wet).
  pub moisture: f32,
  /// Height in metres where frost starts: the tree line. Far above the
  /// map (1e6 m) where there is none.
  pub frost_floor: f32,
  /// Metres over which frost rises to its full strength, at the snow
  /// line: the same everywhere on a map.
  pub frost_band: f32,
}

/// No frost: a climate for maps without snow.
pub const NO_FROST: f32 = 1.0e6;

/// The soil model's result, one byte per heightmap sample.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SoilField {
  /// Rock exposure, 0 to 255.
  pub rock: Vec<u8>,
  /// Scree below outcrops, 0 to 255.
  pub scree: Vec<u8>,
  /// The talus field (see [`talus_byte`]).
  pub talus: Vec<u8>,
}

/// Pack the boulder field: the top four bits hold the probability of a
/// boulder per candidate out of [`MAX_TALUS_PROBABILITY`], the bottom four
/// how far down its talus cone the sample lies (0 at the outcrop's foot,
/// 15 at the cone's end).
pub fn talus_byte(probability: f32, position: f32) -> u8 {
  let level = (probability / MAX_TALUS_PROBABILITY * 15.0)
    .round()
    .clamp(0.0, 15.0) as u8;
  let place = (position * 15.0).round().clamp(0.0, 15.0) as u8;
  level << 4 | place
}

/// Unpack [`talus_byte`]: the probability and the position down the cone.
pub fn talus_parts(byte: u8) -> (f32, f32) {
  (
    f32::from(byte >> 4) / 15.0 * MAX_TALUS_PROBABILITY,
    f32::from(byte & 15) / 15.0,
  )
}

/// Smooth fields of the map, evaluated every `step` samples and read
/// bilinearly between: the beds' warp and cropping and the climate vary
/// over hundreds of metres, so a lattice a few tens of metres apart holds
/// them.
struct Coarse {
  columns: usize,
  step: u32,
  values: Vec<[f32; 4]>,
}

impl Coarse {
  fn new(width: u32, height: u32, step: u32, value: &dyn Fn(u32, u32) -> [f32; 4]) -> Self {
    let (columns, rows) = ((width - 1) / step + 2, (height - 1) / step + 2);
    let mut values = Vec::with_capacity((columns * rows) as usize);

    for row in 0..rows {
      for column in 0..columns {
        values.push(value(column * step, row * step));
      }
    }

    Self {
      columns: columns as usize,
      step,
      values,
    }
  }

  fn at(&self, x: u32, y: u32) -> [f32; 4] {
    let scale = 1.0 / self.step as f32;
    let (tx, ty) = (
      (x % self.step) as f32 * scale,
      (y % self.step) as f32 * scale,
    );
    let index = (y / self.step) as usize * self.columns + (x / self.step) as usize;
    let (a, b) = (self.values[index], self.values[index + 1]);
    let (c, d) = (
      self.values[index + self.columns],
      self.values[index + self.columns + 1],
    );
    let mut out = [0.0; 4];

    for k in 0..4 {
      let top = a[k] + (b[k] - a[k]) * tx;
      let bottom = c[k] + (d[k] - c[k]) * tx;
      out[k] = top + (bottom - top) * ty;
    }

    out
  }
}

/// Slope in degrees from a unit normal.
fn slope_of(normal: Vec3) -> f32 {
  normal[1].clamp(-1.0, 1.0).acos().to_degrees()
}

/// Soil depth, in metres, at one sample. `convexity` is the mean
/// convexity index over the two scales (positive on crests, negative in
/// hollows: the height above the surrounding ring per metre out),
/// `drainage` the upstream area in samples, and `hardness` the bed's
/// hardness there.
pub fn soil_depth(site: Site, slope: f32, convexity: f32, drainage: f32, hardness: f32) -> f32 {
  let base = 1.6 * (0.55 + 0.9 * site.moisture.clamp(0.0, 1.0));
  let slope_loss = 2.2
    * smoothstep((slope - SLOPE_LOSS_DEGREES.0) / (SLOPE_LOSS_DEGREES.1 - SLOPE_LOSS_DEGREES.0));
  let convexity_loss = if convexity > 0.0 {
    1.2 * smoothstep(convexity / CONVEXITY_SCALE)
  } else {
    -0.8 * smoothstep(-convexity / CONVEXITY_SCALE)
  };
  // Open hillslopes gather little: soil deepens from about 20 samples
  // upstream, and most on valley floors.
  let accumulation = smoothstep((drainage.max(1.0).ln() - 3.0) / 6.0);
  let frost = site.frost.clamp(0.0, 1.0);
  let strata = 1.5 * hardness * smoothstep((slope - 22.0) / 8.0);
  (base - slope_loss - convexity_loss + accumulation - frost - strata).clamp(0.0, MAX_SOIL_METRES)
}

/// Rock exposure for a soil depth, 0 to 1, with `rockiness` scaling how
/// deep the soil effectively is.
pub fn exposure(depth: f32, rockiness: f32) -> f32 {
  if rockiness <= 0.0 {
    return 0.0;
  }

  let depth = depth / rockiness;
  1.0 - smoothstep((depth - SOIL_BARE_METRES) / (SOIL_COVERS_METRES - SOIL_BARE_METRES))
}

/// The steepest downhill neighbour of sample `index`, if any.
fn steepest_below(map: &HeightMap, index: usize) -> Option<usize> {
  let width = map.metadata.width as i32;
  let height = map.metadata.height as i32;
  let (x, y) = (index as i32 % width, index as i32 / width);
  let here = map.heights[index];
  let mut best = None;
  let mut best_drop = 0.0;

  for dy in -1..=1 {
    for dx in -1..=1 {
      let (nx, ny) = (x + dx, y + dy);

      if (dx == 0 && dy == 0) || nx < 0 || ny < 0 || nx >= width || ny >= height {
        continue;
      }

      let neighbour = (ny * width + nx) as usize;
      let length = if dx != 0 && dy != 0 {
        std::f32::consts::SQRT_2
      } else {
        1.0
      };
      let drop = (here - map.heights[neighbour]) / length;

      if drop > best_drop {
        best_drop = drop;
        best = Some(neighbour);
      }
    }
  }

  best
}

/// The soil model over the whole map. `normals` are the per-sample
/// normals, `drainage` the upstream areas, and `climate` the climate at a
/// sample.
pub fn soil_field(
  map: &HeightMap,
  normals: &[Vec3],
  drainage: &DrainageArea,
  options: &SoilOptions,
  climate: &dyn Fn(u32, u32) -> Climate,
) -> SoilField {
  let (width, height) = (map.metadata.width, map.metadata.height);
  let count = (width as usize) * (height as usize);

  if count == 0 || normals.len() != count || map.heights.len() != count {
    return SoilField {
      rock: vec![0; count],
      scree: vec![0; count],
      talus: vec![0; count],
    };
  }

  let metres = map.metadata.metres_per_sample.max(0.001);
  let half = [
    (width as f32 - 1.0) * metres * 0.5,
    (height as f32 - 1.0) * metres * 0.5,
  ];
  let edge_seed = options.strata.seed ^ 0x3ed9_e5c1;
  let at = |x: i32, y: i32| {
    let x = x.clamp(0, width as i32 - 1) as u32;
    let y = y.clamp(0, height as i32 - 1) as u32;
    map.heights[(y * width + x) as usize]
  };
  // The height above the ring `r` samples out, per metre out: a shoulder
  // or crest is positive, a hollow negative.
  let convexity = |x: i32, y: i32, r: i32| {
    let ring = (at(x - r, y) + at(x + r, y) + at(x, y - r) + at(x, y + r)) * 0.25;
    (at(x, y) - ring) / (r as f32 * metres)
  };
  let world = |x: u32, y: u32| [x as f32 * metres - half[0], y as f32 * metres - half[1]];
  let strata = &options.strata;
  let frost_band = climate(0, 0).frost_band.max(1.0);
  let beds = Coarse::new(
    width,
    height,
    (32.0 / metres).ceil().max(1.0) as u32,
    &|x, y| {
      let [wx, wz] = world(x, y);
      let here = climate(x.min(width - 1), y.min(height - 1));
      [
        strata.warp(wx, wz),
        strata.cropping(wx, wz),
        here.moisture,
        here.frost_floor,
      ]
    },
  );
  let mut rock = vec![0.0f32; count];
  let mut slopes = vec![0.0f32; count];

  for y in 0..height {
    for x in 0..width {
      let index = (y * width + x) as usize;

      if map.no_data[index] {
        continue;
      }

      let h = map.heights[index];
      let (xi, yi) = (x as i32, y as i32);
      let convex = 0.5 * (convexity(xi, yi, 3) + convexity(xi, yi, 9));
      let slope = slope_of(normals[index]);
      slopes[index] = slope;
      let [warp, cropping, moisture, frost_floor] = beds.at(x, y);
      let site = Site {
        moisture,
        frost: ((h - frost_floor) / frost_band).clamp(0.0, 1.0),
      };

      // Gathering water only deepens the soil, and no bed is harder than
      // where it crops out, so where the soil is too deep for any edge
      // noise to bare it even then the sample stays covered: most ground.
      if soil_depth(site, slope, convex, 1.0, cropping)
        >= SOIL_COVERS_METRES * options.rockiness + 0.3
      {
        continue;
      }

      let [wx, wz] = world(x, y);
      let hardness = Strata::hardness(strata.phase_warped(wx, wz, h, warp)) * cropping;
      let depth = soil_depth(site, slope, convex, drainage.at(x, y), hardness);
      // Ragged, lobed outcrop edges: a noise three samples across, up to
      // 0.3 m either way. It matters only near the threshold, so it is
      // left out where the soil is deeper.
      if depth < SOIL_COVERS_METRES * options.rockiness + 0.3 {
        let edge = value_noise(edge_seed, x as f32 / 3.0, y as f32 / 3.0);
        rock[index] = exposure((depth + edge * 0.3).max(0.0), options.rockiness);
      }
    }
  }

  let mut scree = vec![0.0f32; count];
  let mut position = vec![1.0f32; count];
  let mut foot = vec![0.0f32; count];
  let foot_steps = (FOOT_METRES / metres).ceil().max(1.0) as usize;
  // Each sample's steepest downhill neighbour, found when a walk first
  // passes it: `UNKNOWN` until then, `NONE` where there is none.
  const UNKNOWN: u32 = u32::MAX;
  const NONE: u32 = u32::MAX - 1;
  let mut below = vec![UNKNOWN; count];

  for (start, exposed) in rock.iter().enumerate() {
    if *exposed <= OUTCROP_EXPOSURE {
      continue;
    }

    // Stronger outcrops shed their scree further: 2 to 8 samples.
    let steps =
      2 + ((exposed - OUTCROP_EXPOSURE) / (1.0 - OUTCROP_EXPOSURE) * 6.0).round() as usize;
    let mut here = start;
    let mut sliding = true;

    for step in 1..=steps.max(foot_steps) {
      if below[here] == UNKNOWN {
        below[here] = steepest_below(map, here).map_or(NONE, |next| next as u32);
      }

      if below[here] == NONE {
        break;
      }

      here = below[here] as usize;

      if step <= foot_steps && *exposed > FOOT_EXPOSURE {
        foot[here] = foot[here].max(*exposed);
      }

      let slope = slopes[here];
      sliding &= step <= steps && slope > SCREE_SLOPE_DEGREES;

      if sliding {
        // Scree comes to rest at its angle of repose: on steeper ground
        // it slides on and little stays.
        let along = (step - 1) as f32 / steps as f32;
        let resting = 1.0 - smoothstep((slope - SCREE_REPOSE_DEGREES) / 10.0);
        let deposit = exposed * (1.0 - along) * 0.35 * resting;

        if deposit > scree[here] {
          scree[here] = deposit;
          position[here] = step as f32 / steps as f32;
        }
      } else if step >= foot_steps {
        break;
      }
    }
  }

  let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
  let mut field = SoilField {
    rock: Vec::with_capacity(count),
    scree: Vec::with_capacity(count),
    talus: Vec::with_capacity(count),
  };

  for index in 0..count {
    if scree[index] == 0.0 && foot[index] == 0.0 {
      field.rock.push(byte(rock[index]));
      field.scree.push(0);
      field.talus.push(0);
      continue;
    }

    // Scree lies on what is not bare rock. Blocks come to rest on scree
    // and at an outcrop's foot, not on the crag itself, and slide off
    // ground steeper than they can rest on.
    let lying = scree[index] * (1.0 - rock[index]);
    let at_foot = smoothstep((foot[index] - FOOT_EXPOSURE) / (1.0 - FOOT_EXPOSURE) + 0.5)
      * (1.0 - smoothstep((rock[index] - 0.3) / 0.3));
    let resting = 1.0 - smoothstep((slopes[index] - BOULDER_REST_DEGREES) / 6.0);
    let gathered = lying.max(at_foot * 0.8) * resting;
    let probability = 0.35 * gathered * (1.0 + 0.5 * at_foot);
    let place = if lying >= at_foot * 0.8 {
      position[index]
    } else {
      0.0
    };
    field.rock.push(byte(rock[index]));
    field.scree.push(byte(lying));
    field.talus.push(if gathered > 0.2 {
      talus_byte(probability, place)
    } else {
      0
    });
  }

  field
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::terrain::normals::generate_normals;
  use vista_types::TerrainMetadata;

  #[test]
  fn depth_never_falls_with_drainage_or_rises_with_hardness() {
    // `soil_field` skips samples whose depth, with no drainage and the
    // hardest bed there, already covers them; that holds only while more
    // drainage never thins the soil and a softer bed never does either.
    for moisture in [0.0, 0.5, 1.0] {
      for frost in [0.0, 0.5, 1.0] {
        let site = Site { moisture, frost };

        for slope in [0.0, 10.0, 25.0, 40.0, 70.0] {
          for convexity in [-1.0, 0.0, 0.3, 1.0] {
            for cropping in [0.0, 0.5, 1.0] {
              let floor = soil_depth(site, slope, convexity, 1.0, cropping);

              for drainage in [1.0, 50.0, 1.0e5] {
                for hardness in [0.0, 0.5 * cropping, cropping] {
                  assert!(soil_depth(site, slope, convexity, drainage, hardness) >= floor);
                }
              }
            }
          }
        }
      }
    }
  }

  fn map_of(size: u32, metres: f32, height: impl Fn(f32, f32) -> f32) -> HeightMap {
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: metres,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let heights = (0..size * size)
      .map(|i| height((i % size) as f32 * metres, (i / size) as f32 * metres))
      .collect();
    HeightMap::from_values(
      size,
      size,
      heights,
      vec![false; (size * size) as usize],
      metadata,
    )
    .expect("a valid map")
  }

  fn field(map: &HeightMap, options: &SoilOptions) -> SoilField {
    let normals = generate_normals(map);
    let drainage = DrainageArea::d8(map.metadata.width, map.metadata.height, &map.heights);
    let temperate = |_: u32, _: u32| Climate {
      moisture: 0.5,
      frost_floor: NO_FROST,
      frost_band: 100.0,
    };
    soil_field(map, &normals, &drainage, options, &temperate)
  }

  /// Depths over the whole map, as the field computes them before
  /// exposure.
  fn depths(map: &HeightMap) -> Vec<f32> {
    let normals = generate_normals(map);
    let drainage = DrainageArea::d8(map.metadata.width, map.metadata.height, &map.heights);
    let size = map.metadata.width as i32;
    let metres = map.metadata.metres_per_sample;
    let at =
      |x: i32, y: i32| map.heights[(y.clamp(0, size - 1) * size + x.clamp(0, size - 1)) as usize];
    let convexity = |x: i32, y: i32, r: i32| {
      (at(x, y) - (at(x - r, y) + at(x + r, y) + at(x, y - r) + at(x, y + r)) * 0.25)
        / (r as f32 * metres)
    };

    (0..size * size)
      .map(|i| {
        let (x, y) = (i % size, i / size);
        soil_depth(
          Site {
            moisture: 0.5,
            frost: 0.0,
          },
          slope_of(normals[i as usize]),
          0.5 * (convexity(x, y, 3) + convexity(x, y, 9)),
          drainage.at(x as u32, y as u32),
          0.0,
        )
      })
      .collect()
  }

  #[test]
  fn crests_hold_less_soil_than_hollows_and_valley_floors_the_most() {
    // Parallel ridges and valleys 240 m apart, falling gently to the
    // south so the valleys drain.
    let map = map_of(96, 10.0, |x, z| {
      20.0 * (x / 240.0 * std::f32::consts::TAU).cos() + (960.0 - z) * 0.05
    });
    let depth = depths(&map);
    let size = 96usize;
    let row = 60 * size;
    // Crests at x = 0, 240, 480 m (columns 0, 24, 48); valley floors at
    // 120 and 360 m (columns 12 and 36); a hollow between them.
    let crest = depth[row + 24];
    let valley = depth[row + 36];
    let hollow = depth[row + 33];

    assert!(crest < hollow, "{crest} {hollow}");
    assert!(hollow <= valley, "{hollow} {valley}");

    // The valley floor is the deepest soil across the valley.
    let across: Vec<f32> = (24..=48).map(|x| depth[row + x]).collect();
    let deepest = across.iter().copied().fold(0.0, f32::max);
    assert!(valley >= deepest - 1e-4, "{valley} {deepest}");
  }

  #[test]
  fn the_soil_field_is_deterministic() {
    let map = map_of(64, 12.0, |x, z| {
      300.0 * ((x / 300.0).sin() * (z / 250.0).cos()).abs() + 0.2 * x
    });
    let options = SoilOptions {
      rockiness: 1.3,
      strata: Strata::new(LandformKind::Alpine, 7),
    };
    assert_eq!(field(&map, &options), field(&map, &options));
  }

  #[test]
  fn rockiness_zero_keeps_deep_soil_everywhere_and_two_bares_more_rock() {
    let map = map_of(64, 12.0, |x, z| (x - 384.0).hypot(z - 384.0) * -0.7 + 500.0);
    let rock = |rockiness: f32| {
      field(
        &map,
        &SoilOptions {
          rockiness,
          ..SoilOptions::default()
        },
      )
      .rock
      .iter()
      .map(|r| u32::from(*r))
      .sum::<u32>()
    };

    assert_eq!(rock(0.0), 0);
    assert!(rock(2.0) > rock(1.0) && rock(1.0) > 0);
  }

  /// A 30-degree cone 1,500 m across, at 6 m a sample.
  fn cone() -> HeightMap {
    let rise = 30f32.to_radians().tan();
    map_of(256, 6.0, |x, z| {
      (1100.0 - (x - 765.0).hypot(z - 765.0) * rise).max(0.0)
    })
  }

  #[test]
  fn strata_on_a_cone_form_bands_along_the_contours() {
    let map = cone();
    let options = SoilOptions {
      rockiness: 1.0,
      strata: Strata {
        period: 40.0,
        dip: [0.0; 2],
        seed: 3,
        outcrop: 0.25,
      },
    };
    let soil = field(&map, &options);
    let size = 256usize;
    let centre = 765.0 / 6.0;
    let rocky: Vec<bool> = soil.rock.iter().map(|r| *r > 160).collect();
    let mut label = vec![usize::MAX; size * size];
    let mut components = Vec::new();

    // Connected rock components on the cone's flank (away from the summit
    // and the foot).
    for start in 0..size * size {
      let (sx, sy) = ((start % size) as f32, (start / size) as f32);
      let radius = (sx - centre).hypot(sy - centre);

      if !rocky[start] || label[start] != usize::MAX || !(15.0..110.0).contains(&radius) {
        continue;
      }

      let id = components.len();
      let mut stack = vec![start];
      let mut cells = Vec::new();
      label[start] = id;

      while let Some(cell) = stack.pop() {
        cells.push(cell);
        let (x, y) = ((cell % size) as i32, (cell / size) as i32);

        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
          let (nx, ny) = (x + dx, y + dy);

          if nx < 0 || ny < 0 || nx >= size as i32 || ny >= size as i32 {
            continue;
          }

          let n = (ny as usize) * size + nx as usize;
          let r = ((nx as f32) - centre).hypot(ny as f32 - centre);

          if rocky[n] && label[n] == usize::MAX && (15.0..110.0).contains(&r) {
            label[n] = id;
            stack.push(n);
          }
        }
      }

      components.push(cells);
    }

    // Each sizeable component's principal axis against the contour (the
    // tangent round the cone) at its centroid.
    let mut angles = Vec::new();

    for cells in components.iter().filter(|cells| cells.len() >= 12) {
      let n = cells.len() as f32;
      let points: Vec<(f32, f32)> = cells
        .iter()
        .map(|c| ((c % size) as f32, (c / size) as f32))
        .collect();
      let (mx, my) = points
        .iter()
        .fold((0.0, 0.0), |(a, b), (x, y)| (a + x / n, b + y / n));
      let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);

      for (x, y) in &points {
        sxx += (x - mx) * (x - mx);
        syy += (y - my) * (y - my);
        sxy += (x - mx) * (y - my);
      }

      let axis = 0.5 * (2.0 * sxy).atan2(sxx - syy);
      let tangent = (my - centre).atan2(mx - centre) + std::f32::consts::FRAC_PI_2;
      let mut angle = (axis - tangent).rem_euclid(std::f32::consts::PI);

      if angle > std::f32::consts::FRAC_PI_2 {
        angle = std::f32::consts::PI - angle;
      }

      angles.push(angle.to_degrees());
    }

    assert!(!angles.is_empty());
    let mean = angles.iter().sum::<f32>() / angles.len() as f32;
    assert!(mean < 30.0, "mean {mean} over {} components", angles.len());

    // Down one radius, rock comes and goes in at least three bands.
    let mut bands = 0;
    let mut inside = false;

    for step in 15..110 {
      let index = (centre as usize) * size + centre as usize + step;
      let now = rocky[index];

      if now && !inside {
        bands += 1;
      }

      inside = now;
    }

    assert!(bands >= 3, "{bands}");
  }

  #[test]
  fn scree_lies_only_below_rock_on_steep_ground_and_stops_where_it_eases() {
    // A cliff band above a 32-degree talus slope that eases to 12 degrees.
    let map = map_of(96, 4.0, |_, z| {
      let cliff = if z < 100.0 { 600.0 - z * 2.5 } else { 350.0 };
      let talus_start = 100.0;
      let toe = 250.0;

      if z < talus_start {
        cliff
      } else if z < toe {
        350.0 - (z - talus_start) * 32f32.to_radians().tan()
      } else {
        350.0
          - (toe - talus_start) * 32f32.to_radians().tan()
          - (z - toe) * 12f32.to_radians().tan()
      }
    });
    let soil = field(&map, &SoilOptions::default());
    let normals = generate_normals(&map);
    let size = 96usize;
    let mut scree_rows = Vec::new();

    for (index, scree) in soil.scree.iter().enumerate() {
      if *scree == 0 {
        continue;
      }

      let (x, y) = (index % size, index / size);
      assert!(slope_of(normals[index]) > SCREE_SLOPE_DEGREES, "{x} {y}");
      // Rock lies upslope, within the longest walk.
      assert!(
        (1..=8).any(|up| y >= up && soil.rock[index - up * size] as f32 / 255.0 > OUTCROP_EXPOSURE),
        "{x} {y}"
      );
      scree_rows.push(y);
    }

    assert!(!scree_rows.is_empty());
    // Nothing past the toe of the talus, where the slope eases.
    let toe_row = 250 / 4;
    assert!(
      scree_rows.iter().all(|y| *y <= toe_row + 1),
      "{scree_rows:?}"
    );
  }

  #[test]
  fn the_talus_byte_round_trips() {
    for level in 0..16u8 {
      for place in 0..16u8 {
        let byte = level << 4 | place;
        let (probability, position) = talus_parts(byte);
        assert_eq!(talus_byte(probability, position), byte);
      }
    }
  }
}
