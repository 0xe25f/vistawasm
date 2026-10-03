use vista_types::{BiomeKind, GrassOptions};

use crate::maths::hash_noise;
use crate::maths::smoothstep;
use crate::render::flora::{
  unit_from_hash, FloraInstance, FloraVertex, GRASS_STYLE_FERN, GRASS_STYLE_REED, GRASS_STYLE_TUFT,
  GRASS_STYLE_UNDERGROWTH,
};
use crate::render::water::{WetBanks, WET_BANK_RANGE_METRES};
use crate::terrain::biomes::{
  SurfaceSample, MAT_DRY_GRASS, MAT_FOREST_FLOOR, MAT_LUSH_GRASS, MAT_ROCK, MAT_TUNDRA,
};
use crate::terrain::heightmap::HeightMap;

/// One quad's worth of vertices, reused for every one of a grass tuft's
/// three crossed blades.
const GRASS_QUAD: [FloraVertex; 6] = [
  FloraVertex {
    local_offset: [-0.5, 0.0],
    uv: [0.0, 0.0],
  },
  FloraVertex {
    local_offset: [0.5, 0.0],
    uv: [1.0, 0.0],
  },
  FloraVertex {
    local_offset: [-0.5, 1.0],
    uv: [0.0, 1.0],
  },
  FloraVertex {
    local_offset: [0.5, 0.0],
    uv: [1.0, 0.0],
  },
  FloraVertex {
    local_offset: [0.5, 1.0],
    uv: [1.0, 1.0],
  },
  FloraVertex {
    local_offset: [-0.5, 1.0],
    uv: [0.0, 1.0],
  },
];

/// Three copies of [`GRASS_QUAD`] (18 vertices) shared by every grass
/// instance: reeds draw all three, 60 degrees apart; tufts within 15 m
/// the first two, 90 degrees apart; and tufts beyond it the first one.
///
/// Near the camera grass is never camera-facing: the vertex shader
/// orients each group of six vertices at a different fixed world angle
/// (plus a per-instance random offset), which gives real parallax as the
/// camera moves — important for foliage this close to the lens, where a
/// flat camera-facing cutout would be obvious. Beyond 15 m, where
/// parallax is slight, one card turned to the camera does.
pub const GRASS_BASE_TUFT: [FloraVertex; 18] = [
  GRASS_QUAD[0],
  GRASS_QUAD[1],
  GRASS_QUAD[2],
  GRASS_QUAD[3],
  GRASS_QUAD[4],
  GRASS_QUAD[5],
  GRASS_QUAD[0],
  GRASS_QUAD[1],
  GRASS_QUAD[2],
  GRASS_QUAD[3],
  GRASS_QUAD[4],
  GRASS_QUAD[5],
  GRASS_QUAD[0],
  GRASS_QUAD[1],
  GRASS_QUAD[2],
  GRASS_QUAD[3],
  GRASS_QUAD[4],
  GRASS_QUAD[5],
];

/// Clamp grass instances to a device or implementation limit.
pub fn clamp_grass_instances(options: &GrassOptions, device_limit: u32) -> u32 {
  if !options.enabled {
    return 0;
  }

  options.max_instances.min(device_limit)
}

/// Candidate grid resolution used when placing reeds. Reeds only grow
/// within a few metres of still water, so one candidate per sample is
/// enough.
const MAX_REED_SAMPLES_PER_SIDE: u32 = 768;

/// Tundra tufts grow at this fraction of the density of a meadow.
const TUNDRA_GRASS_DENSITY: f32 = 0.4;

/// Tundra tufts are this fraction of the height of meadow grass.
pub const TUNDRA_GRASS_HEIGHT: f32 = 0.4;

/// Reeds grow within this distance of still or slow water, in metres, or
/// on the first samples from the shore where samples are further apart,
/// as long as the wet-bank field still measures that far.
const REED_METRES: f32 = 3.0;

/// Reeds need a mean temperature above this, in °C.
const REED_CELSIUS: f32 = 4.0;

/// Reeds stand this far apart along each bank of a brook, in metres,
/// before thinning by density.
const REED_SPACING: f32 = 1.5;

/// Grass stands at least this high above sea level, in metres.
pub const GRASS_WATER_LINE: f32 = 0.5;

/// Within this distance of water, in metres, grass grows denser and
/// greener.
pub const GRASS_NEAR_WATER_METRES: f32 = 12.0;

/// A riparian band's tufts grow within this distance of a channel's edge,
/// in metres, leaning over the water; its tall herbs from 0.3 m out to
/// [`HERB_METRES`].
pub const TUFT_METRES: f32 = 0.6;
/// See [`TUFT_METRES`].
pub const HERB_METRES: f32 = 3.0;
/// The chance of a riparian tuft at each grass point where the band's
/// density is 1.
pub const RIPARIAN_TUFTS: f32 = 0.5;
/// The chance of a tall herb, fern or sedge at each grass point where the
/// band's density is 1, doubled in full shade.
pub const RIPARIAN_HERBS: f32 = 0.12;

/// Ferns and undergrowth grow at this fraction of the meadow's density,
/// under full canopy.
pub const FLOOR_DENSITY: f32 = 0.35;

/// Steepest ground grass grows on, in degrees. It thins smoothly from
/// 60 % of this slope (see [`slope_fade`]).
pub const MAX_PLANTING_SLOPE: f32 = 50.0;

/// Radius of the ground a tuft covers, per unit of its scale: its blades
/// span 1.3 x its scale and taper, so they shade about 70 % of that
/// half-span seen from above.
pub const TUFT_COVER_RADIUS: f32 = 0.45;

/// The share of the ground an ideal meadow's tufts cover within the
/// full-cover radius at slider value `d`: tufts of average scale 0.8
/// scattered at random over the lattice, each covering a disc of
/// [`TUFT_COVER_RADIUS`] x its scale.
pub fn meadow_cover(d: f32) -> f32 {
  use crate::render::lattice::{grass_height, grass_probability, GRASS_PITCH};
  let tufts = grass_probability(d) / (GRASS_PITCH * GRASS_PITCH);
  let radius = TUFT_COVER_RADIUS * 0.8 * grass_height(d);
  1.0 - (-tufts * std::f32::consts::PI * radius * radius).exp()
}

/// Streamed tufts hand over to the ground's grass sheen from here, in
/// metres from the camera: 1.5 times the full-cover radius `radius`, and
/// at least 0.3 times the grass view distance `view` (66 m at the default
/// 220 m), so full cover reaches that far however small the slot budget
/// makes the radius; at most 0.9 times the view distance. Within it,
/// tufts thinned beyond the radius are widened to keep the cover.
pub fn handover_start(radius: f32, view: f32) -> f32 {
  (1.5 * radius).max(0.3 * view).min(0.9 * view)
}

/// The share of the tufts that are drawn `distance` metres from the
/// camera, between the handover's `start` and the view distance `view`:
/// the cull keeps tufts by rank below it (`grass_generate.wgsl`), and
/// their vertices dither out over the last 30 % of the view distance
/// (`grass_instances.wgsl`). The ground's grass sheen takes over what they
/// give up (see [`sheen_share`]).
pub fn tuft_share(distance: f32, start: f32, view: f32) -> f32 {
  let handover = 1.0 - smoothstep((distance - start) / (view - start).max(1.0));
  let dither = 1.0 - smoothstep((distance - 0.7 * view) / (0.3 * view).max(1.0));
  handover * dither
}

/// How much more ground a meadow's tufts hide seen at a slant: their
/// blades' side area over their cover disc's area, from their height
/// (0.78 of a tuft's scale on average), their cover radius
/// ([`TUFT_COVER_RADIUS`]) and how much of a quad the tapered blades fill
/// (about 0.35).
pub const TUFT_SIDE_AREA: f32 = 2.0 * 0.78 / (std::f32::consts::PI * TUFT_COVER_RADIUS) * 0.35;

/// The share of the ground a meadow of cover `cover` (seen from straight
/// above, see [`meadow_cover`]) hides from a view ray at `sine` (the sine
/// of its angle to the ground): from above, only the cover; at a slant,
/// the blades' sides as well, so near the horizon nearly all of it.
/// `side` scales the sides' share (see [`thinned_side`]).
pub fn apparent_cover(cover: f32, sine: f32, side: f32) -> f32 {
  let sine = sine.clamp(0.02, 1.0);
  let cotangent = (1.0 - sine * sine).sqrt() / sine;
  1.0 - (1.0 - cover.clamp(0.0, 0.999)).powf(1.0 + TUFT_SIDE_AREA * side * cotangent)
}

/// How much of the full meadow's side area the thinned tufts keep
/// `distance` metres from the camera, for a full-density radius
/// `radius`: thinning keeps their cover from above by growing them, but
/// past [`crate::render::lattice::MAX_GROWTH`] they only widen, so they
/// show less side for the ground they cover.
pub fn thinned_side(distance: f32, radius: f32) -> f32 {
  (crate::render::lattice::MAX_GROWTH * radius / distance.max(1.0)).min(1.0)
}

/// How far the ground under a meadow takes on the tufts' look, 0 to 1,
/// where `share` of the tufts are drawn (see [`tuft_share`]): the full
/// meadow would hide `full` of the ground (see [`apparent_cover`]), and
/// the thinned tufts, all of them drawn, `thinned`. The drawn tufts hide
/// `1 - (1 - thinned)^share`, and the ground between them makes up the
/// rest, so tufts and sheen together always look like the full meadow: no
/// ring where the tufts end, from any height.
pub fn sheen_share(full: f32, thinned: f32, share: f32) -> f32 {
  let drawn = 1.0 - (1.0 - thinned).max(0.0).powf(share.clamp(0.0, 1.0));
  ((full - drawn) / (1.0 - drawn).max(1e-4)).clamp(0.0, 1.0)
}

/// How much of its density grass keeps on a slope: all of it up to 60 %
/// of [`MAX_PLANTING_SLOPE`], none at it, and a smooth fade between.
/// Tufts drop out by rank as it falls, so the fade is dithered, never a
/// hard band along the contours.
pub fn slope_fade(slope_degrees: f32) -> f32 {
  1.0 - smoothstep((slope_degrees - 0.6 * MAX_PLANTING_SLOPE) / (0.4 * MAX_PLANTING_SLOPE))
}

/// Bake the grass texture the grass generator reads, one texel per
/// texel of `ground`:
///
/// - r: how readily grass grows, from the ground's materials: lush grass,
///   dry grass (0.9), leaf litter (0.25) and tundra (0.4); 0 on glacier;
/// - g: the dry share of the grass, for its colour;
/// - b: 255 where tundra is the ground, for short ochre tufts; elsewhere
///   the rock's share of the ground out of 127 (see [`rock_share`]);
/// - a: soil for ferns and undergrowth under canopy: 0 on rock, sand,
///   snow, ice and mud.
pub fn bake_grass(
  map: &HeightMap,
  surface: &[SurfaceSample],
  ground: &crate::render::vegetation::GroundData,
) -> Vec<[u8; 4]> {
  let texels = (ground.width * ground.height) as usize;

  if surface.len() != map.heights.len() {
    return vec![[0; 4]; texels];
  }

  let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;

  (0..texels)
    .map(|texel| {
      let index = ground.sample_of(map, texel);
      let sample = &surface[index];

      if map.no_data[index] || sample.is_glacier() {
        return [0; 4];
      }

      let (lush, dry) = (sample.weight(MAT_LUSH_GRASS), sample.weight(MAT_DRY_GRASS));
      let (litter, tundra) = (sample.weight(MAT_FOREST_FLOOR), sample.weight(MAT_TUNDRA));
      let accept = lush + dry * 0.9 + litter * 0.25 + tundra * TUNDRA_GRASS_DENSITY;
      let dry_share = if lush + dry > 0.0 {
        dry / (lush + dry)
      } else {
        0.0
      };
      let tundra_style = sample.is_tundra() && tundra >= lush + dry;
      [
        byte(accept),
        byte(dry_share),
        if tundra_style {
          255
        } else {
          (sample.weight(MAT_ROCK) * 127.0).round() as u8
        },
        byte(lush + dry + litter),
      ]
    })
    .collect()
}

/// Reeds beside water: within [`REED_METRES`] of still or slow water in
/// temperate and warm climates (from the wet-bank field), and along both
/// true banks of every brook, streams narrower than a heightmap sample
/// (see `RiverNetwork::brooks`), whatever the sample spacing. The GPU
/// grows every other tuft (`grass_generate.wgsl`); reeds stay here
/// because brook reeds follow the streams' own polylines.
pub fn build_reed_instances(
  map: &HeightMap,
  surface: &[SurfaceSample],
  wet: Option<&WetBanks>,
  brooks: &[Vec<[f32; 3]>],
  options: &GrassOptions,
  density_scale: f32,
) -> Vec<FloraInstance> {
  // Reeds are as dense as the old maximum at most: denser grass does not
  // crowd the shore further.
  let density = (options.density * density_scale).clamp(0.0, 1.0);
  let (width, height) = (map.metadata.width, map.metadata.height);

  if !options.enabled
    || density <= 0.0
    || options.max_instances == 0
    || width < 2
    || height < 2
    || surface.len() != map.heights.len()
  {
    return Vec::new();
  }

  let stride = (width.max(height).saturating_sub(1)
    / (MAX_REED_SAMPLES_PER_SIDE.saturating_sub(1)).max(1))
  .max(1);
  let metres = map.metadata.metres_per_sample.max(0.001);
  let half = [
    (width as f32 - 1.0) * metres * 0.5,
    (height as f32 - 1.0) * metres * 0.5,
  ];
  let seed = options.seed_offset;
  let water_line = map.metadata.sea_level_metres + GRASS_WATER_LINE;
  let mut reeds = Vec::new();

  if let Some(wet) = wet {
    let shore = wet.stride as f32 * metres * 0.5 + 0.5;

    for y in (0..height).step_by(stride as usize) {
      for x in (0..width).step_by(stride as usize) {
        let index = (y * width + x) as usize;
        let sample = &surface[index];
        let (_, still) = wet.at(x, y);
        // The field saturates at its range, which reads as "far", not
        // "near".
        let near = still > 0.0 && still < WET_BANK_RANGE_METRES && still <= REED_METRES.max(shore);

        if map.no_data[index]
          || map.heights[index] <= water_line
          || !near
          || sample.is_glacier()
          || sample.celsius() <= REED_CELSIUS
        {
          continue;
        }

        let roll = |salt: u64| unit_from_hash(hash_noise(seed ^ salt, x as i32, y as i32));

        if roll(0) > density * 0.9 {
          continue;
        }

        let jitter = [roll(0x2545_f491) - 0.5, roll(0x38b3_4ae5) - 0.5];
        let span = stride as f32 * metres;
        reeds.push(FloraInstance {
          position: [
            x as f32 * metres - half[0] + jitter[0] * span,
            map.heights[index],
            y as f32 * metres - half[1] + jitter[1] * span,
          ],
          scale: 1.4 + roll(0x0a2b_c3d4) * 0.8,
          tint: roll(0x5f2e_1d0c),
          dryness: 0.0,
          style: GRASS_STYLE_REED,
        });
      }
    }
  }

  add_brook_reeds(&mut reeds, map, surface, brooks, density, seed);
  reeds.truncate(options.max_instances as usize);
  reeds
}

/// Reeds along both banks of each brook, within [`REED_METRES`] of its
/// true edge, where the climate is warm enough.
fn add_brook_reeds(
  candidates: &mut Vec<FloraInstance>,
  map: &HeightMap,
  surface: &[SurfaceSample],
  brooks: &[Vec<[f32; 3]>],
  density: f32,
  seed: u64,
) {
  let metres = map.metadata.metres_per_sample.max(0.001);
  let (width, height) = (map.metadata.width, map.metadata.height);
  let half = [(width as f32 - 1.0) * 0.5, (height as f32 - 1.0) * 0.5];

  for (index, run) in brooks.iter().enumerate() {
    let mut along = 0.0f32;

    for pair in run.windows(2) {
      let (a, b) = (pair[0], pair[1]);
      let length = crate::maths::length2(b[0] - a[0], b[1] - a[1]);

      if length < 1e-4 {
        continue;
      }

      let tangent = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
      let mut t = along.rem_euclid(REED_SPACING);

      while t < length {
        for side in [-1.0f32, 1.0] {
          let key = ((along + t) * 100.0) as i32;
          let lane = index as i32 * 2 + i32::from(side > 0.0);
          let roll = |salt: u64| unit_from_hash(hash_noise(seed ^ salt, key, lane));

          if roll(0x7a3d_91c1) > density * 0.8 {
            continue;
          }

          let out = a[2] + roll(0x1b87_3593) * REED_METRES;
          let x = a[0] + tangent[0] * t - tangent[1] * side * out;
          let z = a[1] + tangent[1] * t + tangent[0] * side * out;
          let (sx, sy) = (x / metres + half[0], z / metres + half[1]);
          let sample = (sy.round().clamp(0.0, half[1] * 2.0) as u32 * width
            + sx.round().clamp(0.0, half[0] * 2.0) as u32) as usize;
          let warm = surface
            .get(sample)
            .is_some_and(|sample| !sample.is_glacier() && sample.celsius() > REED_CELSIUS);

          if warm && !map.no_data[sample] {
            candidates.push(FloraInstance {
              position: [x, crate::terrain::channels::height_at(map, sx, sy), z],
              scale: 1.4 + roll(0x0a2b_c3d4) * 0.8,
              tint: roll(0x5f2e_1d0c),
              dryness: 0.0,
              style: GRASS_STYLE_REED,
            });
          }
        }

        t += REED_SPACING;
      }

      along += length;
    }
  }
}

/// The rock's share of the ground from a grass texel's blue channel: 0
/// on tundra.
pub fn rock_share(blue: u8) -> f32 {
  if blue > 127 {
    0.0
  } else {
    f32::from(blue) / 127.0
  }
}

/// How much thicker turf grows at the lip of a rock outcrop, from the
/// rock's share there (read between texels): up to 1.2 times more on the
/// soil side of the edge, where turf overhangs the rock, and none on the
/// rock.
pub fn edge_turf(rock: f32) -> f32 {
  1.2 * smoothstep((rock - 0.2) / 0.2) * (1.0 - smoothstep((rock - 0.45) / 0.1))
}

/// What every grass point needs beyond the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrassRules {
  /// Lattice seed ([`crate::render::lattice::lattice_seed`]) with the
  /// grass salt.
  pub seed: u32,
  /// The share of points that grow a tuft on ideal meadow (see
  /// [`crate::render::lattice::grass_probability`]).
  pub probability: f32,
  /// `canopy_cover` per unit of cover share: trees' crown area per
  /// square metre at the tree density, or 0 without a forest floor.
  pub canopy: f32,
  /// How tall meadow grass grows (see
  /// [`crate::render::lattice::grass_height`]).
  pub height: f32,
  /// The boulders' lattice seed, when there are boulders to leave room
  /// for.
  pub boulders: Option<u32>,
}

impl GrassRules {
  /// The rules for grass options at an effective density `d` (0 to 4),
  /// under trees at an effective density `trees` (0 to 4, 0 without
  /// trees).
  pub fn new(options: &GrassOptions, d: f32, trees: f32) -> Self {
    Self {
      seed: crate::render::lattice::lattice_seed(options.seed_offset)
        ^ crate::render::lattice::GRASS_SALT,
      probability: crate::render::lattice::grass_probability(d),
      height: crate::render::lattice::grass_height(d),
      canopy: if options.forest_floor {
        crate::render::lattice::canopy_density(trees)
      } else {
        0.0
      },
      boulders: None,
    }
  }
}

/// The chance a grass point at world `(x, z)` grows a plant, before the
/// water, channel and boulder exclusions: meadow grass, and ferns and
/// undergrowth under canopy, with `slope` from [`slope_fade`]; and how
/// near water it is, 0 to 1.
pub struct GrassChance {
  /// The chance of a meadow tuft.
  pub meadow: f32,
  /// The chance of a fern or undergrowth under canopy.
  pub floor: f32,
  /// Nearness to water, 0 to 1.
  pub near: f32,
}

/// See [`GrassChance`]. [`tuft_at`] and the grass density map read it.
pub fn grass_chance(
  ground: &crate::render::vegetation::GroundData,
  rules: &GrassRules,
  x: f32,
  z: f32,
  slope: f32,
) -> GrassChance {
  let texel = ground.nearest(x, z);
  let accept = ground.bilinear(x, z, |texel| f32::from(ground.grass[texel][0]) / 255.0);
  let water = ground.water_distance(x, z);
  let near = (1.0 - smoothstep(water / GRASS_NEAR_WATER_METRES))
    .max(f32::from(ground.banks[texel][2]) / 255.0);
  let red = f32::from(ground.cover[texel][0]);
  let shade = crate::render::lattice::canopy_cover(red, rules.canopy);
  let rock = ground.bilinear(x, z, |texel| rock_share(ground.grass[texel][2]));
  let mask = ground.grass_multiplier(x, z);
  let meadow = rules.probability
    * (accept * (1.0 + 0.6 * near)).min(1.0)
    * (1.0 - shade)
    * slope
    * (1.0 + edge_turf(rock))
    * mask;
  let floor = rules.probability * FLOOR_DENSITY * shade * f32::from(ground.grass[texel][3]) / 255.0
    * slope
    * mask;
  GrassChance {
    meadow,
    floor,
    near,
  }
}

/// The riparian band's grass at world `(x, z)`, from the channel nearest
/// it (see `water::riparian_band`): the chance of a tuft leaning over the
/// water within [`TUFT_METRES`] of its edge, the chance of a tall herb,
/// fern or sedge from 0.3 m out to [`HERB_METRES`] (denser in the shade
/// `shade`, 0 to 1), and the unit direction (x, z) towards the water.
/// None on snow, and none where the wet-bank field puts the water
/// further than the herbs reach, so most points never search the
/// channels. `riparian_grass` in `grass_generate.wgsl`, line for line.
pub fn riparian_grass(
  ground: &crate::render::vegetation::GroundData,
  bins: &[u32],
  x: f32,
  z: f32,
  shade: f32,
) -> [f32; 4] {
  let texel = ground.nearest(x, z);

  if ground.banks[texel][2] == 0
    || ground.surface[texel][2] > 127
    || ground.water_distance(x, z) > HERB_METRES + 0.75 * ground.texel_metres
  {
    return [0.0; 4];
  }

  let Some((edge, base)) = crate::render::vegetation::near_channel(bins, x, z, HERB_METRES) else {
    return [0.0; 4];
  };
  let float = |index: usize| f32::from_bits(bins[index]);
  let band = float(base + 9) * ground.grass_multiplier(x, z);
  let tufts = if edge >= 0.0 {
    band * RIPARIAN_TUFTS * (1.0 - smoothstep((edge - 0.4) / 0.2))
  } else {
    0.0
  };
  let herbs = band
    * RIPARIAN_HERBS
    * smoothstep((edge - 0.3) / 0.2)
    * (1.0 - smoothstep((edge - 2.5) / 0.5))
    * (1.0 + shade);
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
  let to = [a[0] + d[0] * t - x, a[1] + d[1] * t - z];
  let distance = crate::maths::length2(to[0], to[1]).max(1e-6);
  [tufts, herbs, to[0] / distance, to[1] / distance]
}

/// Expected riparian tufts and herbs per texel at every rank (see
/// `vegetation::band_mass`): each bank's band of tufts and of herbs, at
/// most doubled in shade.
pub fn riparian_mass(
  ground: &crate::render::vegetation::GroundData,
  channels: &[Vec<[f32; 3]>],
  bands: &[Vec<f32>],
) -> Vec<f32> {
  use crate::render::lattice::GRASS_PITCH;
  let per_metre = (TUFT_METRES * RIPARIAN_TUFTS + 2.0 * (HERB_METRES - 0.3) * RIPARIAN_HERBS)
    / (GRASS_PITCH * GRASS_PITCH);
  crate::render::vegetation::band_mass(ground, channels, bands, &|half| {
    (per_metre, half + 0.5 * HERB_METRES)
  })
}

/// The share of the ground that grass, ferns and undergrowth cover at
/// world `(x, z)`, 0 to 1, for the grass density map: the chance each
/// lattice point grows a plant there (at most the rules' probability, as
/// [`tuft_at`] caps it), spread as [`meadow_cover`] spreads it.
pub fn grass_cover_at(
  ground: &crate::render::vegetation::GroundData,
  bins: &[u32],
  rules: &GrassRules,
  x: f32,
  z: f32,
) -> f32 {
  use crate::render::lattice::GRASS_PITCH;

  if !ground.on_map(x, z)
    || ground.height_at(x, z) <= ground.sea + GRASS_WATER_LINE
    || ground.in_water(x, z, 0.0)
    || crate::render::vegetation::in_channel(bins, x, z, false)
  {
    return 0.0;
  }

  let chance = grass_chance(ground, rules, x, z, slope_fade(ground.slope_degrees(x, z)));
  let share =
    (chance.meadow + chance.floor).min(rules.probability * ground.grass_multiplier(x, z).max(1.0));
  let radius = TUFT_COVER_RADIUS * 0.8 * rules.height;
  1.0 - (-share / (GRASS_PITCH * GRASS_PITCH) * std::f32::consts::PI * radius * radius).exp()
}

/// The tuft at grass lattice point `(ix, iz)` whose rank relative to its
/// `p` is below `keep`, if one grows there: `grass_generate.wgsl` decides
/// each point the same way, and grounds it on the drawn mesh. Meadow
/// grass is denser and greener near water and thins under canopy, where
/// ferns and undergrowth take its place; tundra grows short tufts. How
/// readily grass grows is read between texels, so it shades smoothly
/// across material borders, and all of it thins on steep ground (see
/// [`slope_fade`]).
pub fn tuft_at(
  ground: &crate::render::vegetation::GroundData,
  bins: &[u32],
  rules: &GrassRules,
  ix: i32,
  iz: i32,
  keep: f32,
) -> Option<FloraInstance> {
  use crate::render::lattice::{jittered, point_hash, unit, GRASS_PITCH};

  let hash = point_hash(ix, iz, rules.seed);
  let [x, z] = jittered(ix, iz, hash, GRASS_PITCH);

  if !ground.on_map(x, z) {
    return None;
  }

  let texel = ground.nearest(x, z);
  let grass = ground.grass[texel];
  let rank = unit(hash[2]);
  let slope = if rank < rules.probability * ground.grass_multiplier(x, z) * keep {
    slope_fade(ground.slope_degrees(x, z))
  } else {
    0.0
  };
  let GrassChance {
    meadow,
    floor,
    near,
  } = grass_chance(ground, rules, x, z, slope);
  let shade = crate::render::lattice::canopy_cover(f32::from(ground.cover[texel][0]), rules.canopy);
  let [tufts, herbs, toward_x, toward_z] = riparian_grass(ground, bins, x, z, shade);
  let p = meadow + floor + tufts + herbs;

  if p <= 0.0 || rank >= p * keep {
    return None;
  }

  let elevation = ground.height_at(x, z);
  // The wet-bank field is coarse beside a stream narrower than a sample;
  // the band keeps out of the channels by their drawn edges, and out of
  // water only where the field is water all round.
  let water = if rank < tufts + herbs {
    ground.water_distance(x, z) <= 0.0
  } else {
    ground.in_water(x, z, 0.0)
  };

  if elevation <= ground.sea + GRASS_WATER_LINE
    || water
    || crate::render::vegetation::in_channel(bins, x, z, false)
    || rules
      .boulders
      .is_some_and(|seed| crate::render::boulders::under_boulder(ground, bins, seed, x, z, 0.0))
  {
    return None;
  }

  let traits = point_hash(
    ix,
    iz,
    rules.seed ^ crate::render::lattice::TREE_TRAITS_SALT,
  );
  let (size_roll, tint_roll) = (unit(traits[0]), unit(traits[1]));
  let relative = rank / p;
  let surface = ground.surface[texel];
  let (mut style, mut scale, mut dryness) = (
    GRASS_STYLE_TUFT,
    0.5 + size_roll * 0.6,
    f32::from(grass[1]) / 255.0 * (1.0 - 0.7 * near),
  );

  let tundra = grass[2] > 127;

  // The band takes the first ranks: by the water it replaces the meadow.
  if rank < tufts {
    // Riparian tufts lean over the water: the eighth of a turn towards
    // it rides on the dryness, as `2 x (sector + 1)`.
    let sector = (toward_z.atan2(toward_x) / (std::f32::consts::TAU / 8.0)).round() as i32 & 7;
    scale *= 1.3
      * if tundra {
        TUNDRA_GRASS_HEIGHT
      } else {
        rules.height
      };
    dryness += 2.0 * (sector + 1) as f32;
  } else if rank < tufts + herbs && (tundra || surface[3] >= BiomeKind::AlpineTransition as u8) {
    // Above the trees the herbs are sedges.
    scale *= 1.2;
    dryness = 0.3;
  } else if rank < tufts + herbs {
    // Tall herbs and ferns, 0.5 to 1.5 m.
    let fern = surface[1] > 140 && (100..200).contains(&surface[0]);
    style = if fern {
      GRASS_STYLE_FERN
    } else {
      GRASS_STYLE_UNDERGROWTH
    };
    scale = 0.7 + 1.3 * size_roll * if fern { 1.0 } else { 0.6 };
    dryness = 0.0;
  } else if rank >= tufts + herbs + meadow {
    // Ferns in temperate and wet ground, undergrowth in any forest.
    let fern = surface[1] > 140 && (100..200).contains(&surface[0]);
    style = if fern {
      GRASS_STYLE_FERN
    } else {
      GRASS_STYLE_UNDERGROWTH
    };
    scale = if fern {
      0.4 + size_roll * 0.5
    } else {
      0.3 + size_roll * 0.3
    };
    dryness = 0.0;
  } else if tundra {
    // Where tundra is the ground, tufts are short and ochre-green.
    scale *= TUNDRA_GRASS_HEIGHT;
    dryness = 0.5;
  } else {
    scale *= rules.height;
  }

  Some(FloraInstance {
    position: [x, elevation, z],
    scale,
    tint: tint_roll,
    dryness,
    style: style + relative.min(0.999),
  })
}

/// The tufts one streamed grass tile holds, as `grass_generate.wgsl`
/// fills it.
pub fn tile_tufts(
  ground: &crate::render::vegetation::GroundData,
  bins: &[u32],
  rules: &GrassRules,
  tile: [i32; 2],
  keep: f32,
) -> Vec<FloraInstance> {
  use crate::render::vegetation::tile_first;
  let range = |t: i32| tile_first(t, 1600, 35)..tile_first(t + 1, 1600, 35);
  range(tile[1])
    .flat_map(|iz| range(tile[0]).map(move |ix| (ix, iz)))
    .filter_map(|(ix, iz)| tuft_at(ground, bins, rules, ix, iz, keep))
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::render::vegetation::{channel_bins, GroundData};
  use vista_types::TerrainMetadata;

  fn flat_map(size: u32, elevation: f32) -> HeightMap {
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 4.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };

    HeightMap::flat(size, size, elevation, metadata)
  }

  fn grass_options() -> GrassOptions {
    GrassOptions {
      enabled: true,
      density: 1.0,
      seed_offset: 99,
      max_instances: 50_000,
      ..GrassOptions::default()
    }
  }

  fn cold_surface(map: &HeightMap, celsius: f32) -> Vec<SurfaceSample> {
    let options = vista_types::BiomeOptions {
      mean_temperature_celsius: Some(celsius),
      volcanism: 0.0,
      ..vista_types::BiomeOptions::default()
    };
    let normals = crate::terrain::normals::generate_normals(map);
    crate::terrain::biomes::classify_surface(map, &normals, None, &[], &options)
  }

  /// Meadow: lush grass everywhere.
  fn meadow(map: &HeightMap, celsius: f32) -> Vec<SurfaceSample> {
    vec![
      SurfaceSample {
        materials: [120, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        celsius_hundredths: (celsius * 100.0) as i16,
        ..SurfaceSample::default()
      };
      map.heights.len()
    ]
  }

  /// The ground the grass generator reads, with no trees.
  fn ground(map: &HeightMap, surface: &[SurfaceSample], wet: Option<&WetBanks>) -> GroundData {
    let mut ground = GroundData::of(map);
    ground.set_surface(map, surface, wet.unwrap_or(&WetBanks::default()), &[]);
    ground.cover = vec![[0; 4]; surface.len()];
    ground.grass = bake_grass(map, surface, &ground);
    ground
  }

  /// Every tuft the GPU would grow in the tiles over the square of half
  /// width `half` metres round the centre, at full density.
  fn tufts(ground: &GroundData, bins: &[u32], rules: &GrassRules, half: f32) -> Vec<FloraInstance> {
    let tiles = (half / crate::render::lattice::GRASS_TILE_METRES).ceil() as i32;
    (-tiles..tiles)
      .flat_map(|tz| (-tiles..tiles).map(move |tx| [tx, tz]))
      .flat_map(|tile| tile_tufts(ground, bins, rules, tile, 1.0))
      .collect()
  }

  fn rules(options: &GrassOptions) -> GrassRules {
    GrassRules::new(options, options.density, 0.0)
  }

  #[test]
  fn glaciers_grow_no_grass_and_tundra_grows_short_sparse_tufts() {
    let map = flat_map(64, 20.0);
    let bins = channel_bins(&map, None);
    // Dense enough to count: a tuft on most of the meadow's points.
    let options = GrassOptions {
      density: 3.0,
      ..grass_options()
    };
    let glacier = cold_surface(&map, -20.0);
    assert!(glacier.iter().all(|sample| sample.is_glacier()));
    assert!(tufts(&ground(&map, &glacier, None), &bins, &rules(&options), 40.0).is_empty());

    let tundra = cold_surface(&map, 1.0);
    assert!(tundra.iter().all(|sample| sample.is_tundra()));
    let short = tufts(&ground(&map, &tundra, None), &bins, &rules(&options), 40.0);
    let lush = vec![
      SurfaceSample {
        materials: [255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        celsius_hundredths: 1000,
        ..SurfaceSample::default()
      };
      map.heights.len()
    ];
    let meadow = tufts(&ground(&map, &lush, None), &bins, &rules(&options), 40.0);

    assert!(!short.is_empty());
    assert!(
      short.len() * 2 < meadow.len(),
      "{} {}",
      short.len(),
      meadow.len()
    );
    assert!(short
      .iter()
      .all(|tuft| tuft.scale <= 1.1 * TUNDRA_GRASS_HEIGHT && tuft.dryness == 0.5));
  }

  /// A lake down the middle column band of a flat map, and the wet-bank
  /// field for it.
  fn lakeside(size: u32) -> (HeightMap, WetBanks) {
    let map = flat_map(size, 20.0);
    let water: Vec<bool> = (0..size * size).map(|i| (i % size) < 8).collect();
    let wet = WetBanks::build(&map, &water, &water);
    (map, wet)
  }

  #[test]
  fn the_wet_bank_field_measures_from_the_water_edge() {
    let (_, wet) = lakeside(64);
    let metres = 4.0;

    assert_eq!(wet.at(3, 10), (0.0, 0.0));
    // Three samples from the last water sample: 2.5 samples from the edge.
    let (water, still) = wet.at(10, 10);
    assert!((water - 2.5 * metres).abs() < 0.2, "{water}");
    assert_eq!(water, still);
    assert_eq!(wet.at(60, 10).0, 40.0);
  }

  #[test]
  fn grass_is_denser_near_water_and_reeds_grow_by_warm_still_water() {
    let (map, wet) = lakeside(96);
    let bins = channel_bins(&map, None);
    let options = GrassOptions {
      density: 2.0,
      ..grass_options()
    };
    let warm = meadow(&map, 12.0);
    let with_water = tufts(
      &ground(&map, &warm, Some(&wet)),
      &bins,
      &rules(&options),
      190.0,
    );
    let without = tufts(&ground(&map, &warm, None), &bins, &rules(&options), 190.0);
    // Back to sample columns.
    let column = |position: [f32; 3]| (position[0] + 95.0 * 2.0) / 4.0;
    let near = |instances: &[FloraInstance]| {
      instances
        .iter()
        .filter(|i| column(i.position) > 8.5 && column(i.position) < 13.0)
        .count()
    };

    assert!(
      near(&with_water) > near(&without),
      "{} {}",
      near(&with_water),
      near(&without)
    );
    // No tuft grows in the lake, whose edge is half a sample beyond its
    // last sample (to within the distance field's 16 cm steps).
    assert!(with_water.iter().all(|i| column(i.position) >= 7.45));

    let reeds = build_reed_instances(&map, &warm, Some(&wet), &[], &options, 1.0);
    assert!(!reeds.is_empty());
    assert!(reeds.iter().all(|reed| {
      reed.style == GRASS_STYLE_REED
        && reed.scale >= 1.4
        && reed.scale <= 2.2
        && column(reed.position) <= 9.0
    }));

    let cold = meadow(&map, 1.0);
    assert!(build_reed_instances(&map, &cold, Some(&wet), &[], &options, 1.0).is_empty());
  }

  #[test]
  fn reeds_do_not_spread_over_coarse_maps() {
    // Samples 120 m apart: even the first shore sample lies beyond the
    // wet-bank field's range, where it reads as far from water.
    let size = 64;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 120.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let map = HeightMap::flat(size, size, 20.0, metadata);
    let water: Vec<bool> = (0..size * size).map(|i| (i % size) < 8).collect();
    let wet = WetBanks::build(&map, &water, &water);
    let surface = meadow(&map, 12.0);
    let reeds = build_reed_instances(&map, &surface, Some(&wet), &[], &grass_options(), 1.0);
    assert!(reeds.is_empty());

    // The meadow itself still grows grass.
    let bins = channel_bins(&map, None);
    let grass = GrassOptions {
      density: 3.0,
      ..grass_options()
    };
    assert!(!tufts(
      &ground(&map, &surface, Some(&wet)),
      &bins,
      &rules(&grass),
      40.0
    )
    .is_empty());
  }

  #[test]
  fn disabled_or_zero_density_grass_grows_nothing() {
    let map = flat_map(64, 20.0);
    let surface = meadow(&map, 12.0);
    let (_, wet) = lakeside(64);
    let mut options = grass_options();
    options.enabled = false;
    assert!(build_reed_instances(&map, &surface, Some(&wet), &[], &options, 1.0).is_empty());

    options.enabled = true;
    options.density = 0.0;
    assert!(build_reed_instances(&map, &surface, Some(&wet), &[], &options, 1.0).is_empty());
    let bins = channel_bins(&map, None);
    assert!(tufts(&ground(&map, &surface, None), &bins, &rules(&options), 60.0).is_empty());
  }

  #[test]
  fn underwater_terrain_grows_nothing() {
    let map = flat_map(64, -5.0);
    let surface = meadow(&map, 12.0);
    let bins = channel_bins(&map, None);
    let options = GrassOptions {
      density: 3.0,
      ..grass_options()
    };
    assert!(tufts(&ground(&map, &surface, None), &bins, &rules(&options), 60.0).is_empty());
  }

  #[test]
  fn placement_is_deterministic_for_the_same_seed() {
    let map = flat_map(64, 20.0);
    let surface = meadow(&map, 12.0);
    let bins = channel_bins(&map, None);
    let options = GrassOptions {
      density: 2.5,
      ..grass_options()
    };
    let ground = ground(&map, &surface, None);
    let left = tufts(&ground, &bins, &rules(&options), 30.0);
    let right = tufts(&ground, &bins, &rules(&options), 30.0);

    assert!(!left.is_empty());
    assert_eq!(left.len(), right.len());
    for (a, b) in left.iter().zip(right.iter()) {
      assert_eq!(a.position, b.position);
      assert_eq!(a.scale, b.scale);
      assert_eq!(a.tint, b.tint);
    }
  }

  #[test]
  fn reeds_respect_max_instances() {
    let (map, wet) = lakeside(96);
    let mut options = grass_options();
    options.max_instances = 10;
    let reeds = build_reed_instances(&map, &meadow(&map, 12.0), Some(&wet), &[], &options, 1.0);
    assert!(reeds.len() <= 10);
  }

  #[test]
  fn zero_material_grass_weight_blocks_placement() {
    let map = flat_map(64, 20.0);
    let bare = vec![
      SurfaceSample {
        materials: [0, 0, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0],
        ..SurfaceSample::default()
      };
      map.heights.len()
    ];
    let bins = channel_bins(&map, None);
    let options = GrassOptions {
      density: 4.0,
      ..grass_options()
    };
    assert!(tufts(&ground(&map, &bare, None), &bins, &rules(&options), 60.0).is_empty());
  }

  #[test]
  fn full_density_covers_the_meadow() {
    let map = flat_map(64, 20.0);
    let bins = channel_bins(&map, None);
    let options = GrassOptions {
      density: 4.0,
      ..grass_options()
    };
    let grown = tufts(
      &ground(&map, &meadow(&map, 12.0), None),
      &bins,
      &rules(&options),
      16.0,
    );
    // Lush grass at 0.47 acceptance: about half of every lattice point.
    let points = (32.0 / crate::render::lattice::GRASS_PITCH).powi(2);
    assert!(
      grown.len() as f32 > 0.4 * points,
      "{} of {points}",
      grown.len()
    );
  }

  #[test]
  fn default_grass_covers_most_of_a_flat_meadow() {
    let map = flat_map(64, 20.0);
    let bins = channel_bins(&map, None);
    let lush = vec![
      SurfaceSample {
        materials: [255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        celsius_hundredths: 1200,
        ..SurfaceSample::default()
      };
      map.heights.len()
    ];
    let options = GrassOptions {
      density: 0.5,
      ..grass_options()
    };
    let grown = tufts(&ground(&map, &lush, None), &bins, &rules(&options), 16.0);
    // The ground within 6 m of the centre, every 10 cm, is covered where
    // it lies within some tuft's cover disc.
    let near: Vec<&FloraInstance> = grown
      .iter()
      .filter(|tuft| tuft.position[0].abs() < 7.0 && tuft.position[2].abs() < 7.0)
      .collect();
    let (mut covered, mut points) = (0, 0);

    for z in -60..60 {
      for x in -60..60 {
        let (px, pz) = (x as f32 * 0.1, z as f32 * 0.1);
        points += 1;

        if near.iter().any(|tuft| {
          let radius = TUFT_COVER_RADIUS * tuft.scale;
          let (dx, dz) = (tuft.position[0] - px, tuft.position[2] - pz);
          dx * dx + dz * dz < radius * radius
        }) {
          covered += 1;
        }
      }
    }

    let share = covered as f32 / points as f32;
    assert!(share >= 0.7, "{share}");
    assert!(meadow_cover(0.5) >= 0.7);
    assert!(
      (share - meadow_cover(0.5)).abs() < 0.1,
      "{share} {}",
      meadow_cover(0.5)
    );
    // Denser settings cover more.
    assert!(meadow_cover(1.0) > meadow_cover(0.5) && meadow_cover(4.0) > meadow_cover(1.0));
    assert_eq!(meadow_cover(0.0), 0.0);
  }

  /// The ground the cull leaves covered in each 5 m band of distance from
  /// a camera over the centre of a flat, lush meadow, out to `reach`
  /// metres, with default options at `density`: every tuft the stream
  /// generates, thinned, widened and handed over as `grass_generate.wgsl`
  /// culls it, as a disc of [`TUFT_COVER_RADIUS`] times its drawn size.
  /// Also the slots the stream holds, and its `maxInstances`.
  fn cover_by_band(density: f32, reach: f32) -> (Vec<f32>, u32, u32) {
    use crate::render::lattice::{thinning, GRASS_TILE_METRES};
    use crate::render::vegetation::{grass_radius, grass_stream, TileMass};
    let map = flat_map(80, 20.0);
    let bins = channel_bins(&map, None);
    let lush = vec![
      SurfaceSample {
        materials: [255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        celsius_hundredths: 1200,
        ..SurfaceSample::default()
      };
      map.heights.len()
    ];
    let options = GrassOptions {
      density,
      ..GrassOptions::default()
    };
    let ground = ground(&map, &lush, None);
    let rules = rules(&options);
    let view = options.view_distance_metres;
    let budget = vista_types::RenderQualityOptions::default().vegetation();
    let stream = grass_stream(
      TileMass::grass(&ground, &rules, &[]),
      grass_radius(&budget, view),
      view,
      options.max_instances,
    );
    let radius = stream.radius.min(grass_radius(&budget, view));
    let start = handover_start(radius, view);
    // Drawn tufts as cover discs, in 1 m cells.
    let cells = (reach + 8.0).ceil() as i32;
    let side = (2 * cells) as usize;
    let mut discs: Vec<Vec<[f32; 3]>> = vec![Vec::new(); side * side];
    let tiles = ((reach + 4.0) / GRASS_TILE_METRES).ceil() as i32;

    for tz in -tiles..tiles {
      for tx in -tiles..tiles {
        for tuft in tile_tufts(&ground, &bins, &rules, [tx, tz], 1.0) {
          let [x, _, z] = tuft.position;
          let distance = (x * x + z * z + 16.0).sqrt();
          let relative = tuft.style.fract();
          let t = thinning(distance, radius, 0.0);
          let fade = t.fade(relative);
          let kept = tuft_share(distance, start, view) * (t.keep + t.band);

          if fade <= 0.0 || relative >= kept {
            continue;
          }

          let size = TUFT_COVER_RADIUS * tuft.scale * fade * t.height * t.width;
          let (cx, cz) = ((x.floor() as i32 + cells), (z.floor() as i32 + cells));

          if cx >= 0 && cz >= 0 && (cx as usize) < side && (cz as usize) < side {
            discs[cz as usize * side + cx as usize].push([x, z, size]);
          }
        }
      }
    }

    let covered = |px: f32, pz: f32| {
      let (cx, cz) = (px.floor() as i32 + cells, pz.floor() as i32 + cells);
      (cz - 2..=cz + 2).any(|z| {
        (cx - 2..=cx + 2).any(|x| {
          x >= 0
            && z >= 0
            && (x as usize) < side
            && (z as usize) < side
            && discs[z as usize * side + x as usize].iter().any(|disc| {
              let (dx, dz) = (disc[0] - px, disc[1] - pz);
              dx * dx + dz * dz < disc[2] * disc[2]
            })
        })
      })
    };
    // Points on a fine polar grid in each band.
    let bands = (reach / 5.0).ceil() as usize;
    let cover = (0..bands)
      .map(|band| {
        let (mut hit, mut points) = (0, 0);

        for ring in 0..50 {
          let r = band as f32 * 5.0 + (ring as f32 + 0.5) * 0.1;
          let steps = (std::f32::consts::TAU * r / 0.1) as usize;

          for step in 0..steps {
            let angle = step as f32 / steps as f32 * std::f32::consts::TAU;
            points += 1;
            hit += usize::from(covered(r * angle.cos(), r * angle.sin()));
          }
        }

        hit as f32 / points as f32
      })
      .collect();
    (cover, stream.layout.instance_count(), options.max_instances)
  }

  #[test]
  fn default_grass_covers_the_meadow_far_from_the_camera() {
    // A camera 4 m up: at least 70 % cover in every 5 m band out to 60 m
    // at the default density, and out to 40 m at the maximum, within the
    // default slot budget.
    for (density, reach) in [(0.5, 60.0), (4.0, 40.0)] {
      let (cover, slots, cap) = cover_by_band(density, reach);
      assert!(slots <= cap, "density {density}: {slots} slots of {cap}");

      for (band, share) in cover.iter().enumerate() {
        assert!(
          *share >= 0.7,
          "density {density}: {share} cover from {} m",
          band * 5
        );
      }
    }
  }

  #[test]
  fn tufts_and_sheen_hand_over_without_a_ring() {
    use crate::render::lattice::grass_height;
    // Luminances of the full meadow's tufts and of the bare grass ground.
    let (tufts, ground) = (0.06, 0.1);
    let view = GrassOptions::default().view_distance_metres;

    for density in [0.5, 1.0, 4.0] {
      let cover = meadow_cover(density);
      assert!(grass_height(density) >= 1.0);

      for radius in [10.0, 45.0, 120.0] {
        let start = handover_start(radius, view);

        for height in [2.0f32, 10.0, 25.0, 30.0, 60.0] {
          // Combined cover and brightness over the full meadow's, from
          // the camera out to where the sheen has faded to the ground's
          // own grass, in 1 m steps.
          let mut last: Option<(f32, f32)> = None;

          for step in 1..=(4.0 * view) as u32 {
            let across = step as f32;
            let distance = (across * across + height * height).sqrt();
            let apparent = apparent_cover(cover, height / distance, 1.0);
            let thinned = apparent_cover(cover, height / distance, thinned_side(distance, radius));
            let share = tuft_share(distance, start, view);
            let far = 1.0 - smoothstep((distance - 2.0 * view) / (2.0 * view));
            let drawn = 1.0 - (1.0 - thinned).powf(share);
            let sheen = sheen_share(apparent, thinned, share) * far;
            let combined = drawn + (1.0 - drawn) * sheen;
            let full = apparent * if share > 0.999 { 1.0 } else { far };
            let bright = |cover: f32| ground + (tufts - ground) * cover;
            let ratios = (
              combined / apparent,
              bright(combined) / bright(full.max(combined)),
            );

            // Where tufts are drawn at all, the sum is the full meadow.
            if share > 0.0 {
              assert!(
                (combined - apparent).abs() < 1e-3,
                "{height} m up, {across} m: {combined} {apparent}"
              );
            }

            if let Some(previous) = last {
              assert!(
                (ratios.0 - previous.0).abs() <= 0.02 && (ratios.1 - previous.1).abs() <= 0.02,
                "density {density}, radius {radius}, {height} m up, {across} m: {previous:?} to {ratios:?}"
              );
            }

            last = Some(ratios);
          }
        }
      }
    }

    // The shaders use the same side area.
    let common = include_str!("../shaders/common.wgsl");
    assert!(common.contains(&format!("(1.0 + {TUFT_SIDE_AREA:.4} * side * cotangent)")));
  }

  #[test]
  fn grass_fades_smoothly_on_steep_ground() {
    let mut last = slope_fade(0.0);
    assert_eq!(last, 1.0);
    assert_eq!(slope_fade(0.6 * MAX_PLANTING_SLOPE), 1.0);
    assert_eq!(slope_fade(MAX_PLANTING_SLOPE), 0.0);

    // Monotonic, with no hard cut: a tenth of a degree never moves it by
    // more than 1 %.
    for step in 1..=900 {
      let fade = slope_fade(step as f32 * 0.1);
      assert!(fade <= last);
      assert!(last - fade < 0.01, "{} {last} {fade}", step as f32 * 0.1);
      last = fade;
    }

    // On a slope inside the fade, some tufts grow and fewer than on the
    // flat: they thin by rank, a dither, not a band.
    let size = 64;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 4.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let rise = (0.8 * MAX_PLANTING_SLOPE).to_radians().tan() * 4.0;
    let heights = (0..size * size)
      .map(|index| 20.0 + (index % size) as f32 * rise)
      .collect();
    let no_data = vec![false; (size * size) as usize];
    let steep =
      HeightMap::from_values(size, size, heights, no_data, metadata).expect("a valid map");
    let flat = flat_map(size, 20.0);
    let options = GrassOptions {
      density: 1.0,
      ..grass_options()
    };
    let count = |map: &HeightMap| {
      let surface = meadow(map, 12.0);
      tufts(
        &ground(map, &surface, None),
        &channel_bins(map, None),
        &rules(&options),
        16.0,
      )
      .len()
    };
    let (on_slope, on_flat) = (count(&steep), count(&flat));
    assert!(
      on_slope > 0 && on_slope * 2 < on_flat,
      "{on_slope} {on_flat}"
    );
  }

  #[test]
  fn ferns_and_undergrowth_replace_grass_under_canopy() {
    let map = flat_map(64, 20.0);
    let bins = channel_bins(&map, None);
    let options = GrassOptions {
      density: 3.0,
      ..grass_options()
    };
    let surface = vec![
      SurfaceSample {
        materials: [60, 0, 60, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        celsius_hundredths: 1000,
        moisture: 200,
        ..SurfaceSample::default()
      };
      map.heights.len()
    ];
    let mut forest = ground(&map, &surface, None);
    forest.cover = vec![[255, 0, 0, 64]; surface.len()];
    let dense = GrassRules::new(&options, 3.0, 4.0);
    let grown = tufts(&forest, &bins, &dense, 30.0);
    let floor = grown.iter().filter(|t| t.style >= GRASS_STYLE_FERN).count();

    // Under a closed canopy nearly all are ferns (temperate and wet).
    assert!(floor * 10 > grown.len() * 9, "{floor} of {}", grown.len());
    assert!(grown
      .iter()
      .filter(|t| t.style >= GRASS_STYLE_FERN)
      .all(|t| t.style < GRASS_STYLE_UNDERGROWTH && (0.4..=0.9).contains(&t.scale)));
    let open = tufts(&ground(&map, &surface, None), &bins, &dense, 30.0);
    assert!(open.iter().all(|t| t.style < GRASS_STYLE_REED));

    // Without a forest floor, meadow grass thins under trees on its own.
    let plain = GrassRules::new(
      &GrassOptions {
        forest_floor: false,
        ..options
      },
      3.0,
      4.0,
    );
    assert!(tufts(&forest, &bins, &plain, 30.0)
      .iter()
      .all(|t| t.style < GRASS_STYLE_REED));
  }

  #[test]
  fn no_tuft_grows_in_a_drawn_channel() {
    let size = 128;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 12.0,
      ..TerrainMetadata::default()
    };
    let map = HeightMap::flat(size, size, 40.0, metadata);
    let brook: Vec<[f32; 3]> = (0..200)
      .map(|i| {
        let x = -100.0 + i as f32;
        [x, 5.0 * (x / 22.0 * std::f32::consts::TAU).sin(), 1.0]
      })
      .collect();
    let rivers = crate::render::water::RiverNetwork {
      channels: vec![brook.clone()],
      ..Default::default()
    };
    let bins = channel_bins(&map, Some(&rivers));
    let options = GrassOptions {
      density: 4.0,
      ..grass_options()
    };
    let grown = tufts(
      &ground(&map, &meadow(&map, 12.0), None),
      &bins,
      &rules(&options),
      40.0,
    );
    let distance = |x: f32, z: f32| {
      brook
        .windows(2)
        .map(|pair| {
          crate::render::flora::Water {
            a: [pair[0][0], pair[0][1]],
            b: [pair[1][0], pair[1][1]],
            half: [0.0; 2],
            clearance: 0.0,
            stones: [0.0; 2],
            band: 0.0,
          }
          .intrusion(x, z)
        })
        .fold(f32::MIN, f32::max)
        .abs()
    };

    assert!(!grown.is_empty());
    assert!(grown
      .iter()
      .all(|tuft| distance(tuft.position[0], tuft.position[2]) >= 1.0 - 1e-3));
    // Grass grows right to the water's edge.
    assert!(grown
      .iter()
      .any(|tuft| distance(tuft.position[0], tuft.position[2]) < 1.2));
  }

  /// The tufts round a straight brook 2 m wide along z = 0 on a flat
  /// meadow, its riparian band of density `band`, with a wet-bank field
  /// measured from its edge.
  fn riparian_tufts(band: f32) -> Vec<FloraInstance> {
    let map = flat_map(64, 4.0);
    let surface = meadow(&map, 12.0);
    let mut ground = GroundData::of(&map);
    let edge = |texel: usize| {
      let z = (texel as u32 / ground.width) as f32 * ground.texel_metres - ground.half[1];
      ((z.abs() - 1.0).max(0.0) / WET_BANK_RANGE_METRES * 255.0).round() as u8
    };
    let wet = WetBanks {
      width: ground.width,
      height: ground.height,
      stride: ground.stride,
      distance: (0..(ground.width * ground.height) as usize)
        .map(edge)
        .collect(),
      still: Vec::new(),
    };
    ground.set_surface(&map, &surface, &wet, &vec![200; map.heights.len()]);
    ground.cover = vec![[0; 4]; surface.len()];
    ground.grass = bake_grass(&map, &surface, &ground);
    let run: Vec<[f32; 3]> = (0..=40)
      .map(|i| [-100.0 + 5.0 * i as f32, 0.0, 1.0])
      .collect();
    let rivers = crate::render::water::RiverNetwork {
      bands: vec![vec![band; run.len()]],
      channels: vec![run],
      ..Default::default()
    };
    let bins = channel_bins(&map, Some(&rivers));
    tufts(&ground, &bins, &rules(&grass_options()), 30.0)
      .into_iter()
      .filter(|tuft| tuft.position[0].abs() < 90.0)
      .collect()
  }

  #[test]
  fn tufts_lean_over_streams_and_tall_herbs_line_them() {
    let grown = riparian_tufts(1.0);
    let leaning: Vec<_> = grown.iter().filter(|tuft| tuft.dryness >= 2.0).collect();
    let herbs: Vec<_> = grown
      .iter()
      .filter(|tuft| tuft.style >= GRASS_STYLE_FERN)
      .collect();
    assert!(leaning.len() > 200, "{} leaning tufts", leaning.len());
    assert!(herbs.len() > 100, "{} herbs", herbs.len());

    for tuft in &leaning {
      let z = tuft.position[2];
      let edge = z.abs() - 1.0;
      assert!(
        (0.0..=TUFT_METRES).contains(&edge),
        "a leaning tuft {edge} m out"
      );
      // Towards the water: a quarter turn one way or the other.
      let sector = (tuft.dryness * 0.5).floor() as i32 - 1;
      assert_eq!(sector, if z > 0.0 { 6 } else { 2 });
      assert!(tuft.dryness - 2.0 * (sector + 1) as f32 <= 1.0);
    }

    for herb in &herbs {
      let edge = herb.position[2].abs() - 1.0;
      assert!((0.3..=HERB_METRES).contains(&edge), "a herb {edge} m out");
      let height = herb.scale
        * if herb.style < GRASS_STYLE_UNDERGROWTH {
          0.75
        } else {
          1.0
        };
      assert!((0.5..=1.5).contains(&height), "a herb {height} m tall");
    }

    // Without the band the stream is lined with plain meadow.
    let plain = riparian_tufts(0.0);
    assert!(plain
      .iter()
      .all(|tuft| tuft.dryness < 2.0 && tuft.style < GRASS_STYLE_FERN));
    assert!(plain.len() + leaning.len() / 2 < grown.len());
  }

  #[test]
  fn the_generator_grows_the_riparian_band_by_the_same_rules() {
    let wgsl = include_str!("../shaders/grass_generate.wgsl");

    for line in [
      format!("const TUFT_METRES: f32 = {TUFT_METRES};"),
      format!("const HERB_METRES: f32 = {HERB_METRES:.1};"),
      format!("const RIPARIAN_TUFTS: f32 = {RIPARIAN_TUFTS};"),
      format!("const RIPARIAN_HERBS: f32 = {RIPARIAN_HERBS};"),
      "HERB_METRES + 0.75 / m.inverse".to_string(),
      "(1.0 - smoothed((near.edge - 0.4) / 0.2))".to_string(),
      "smoothed((near.edge - 0.3) / 0.2) * (1.0 - smoothed((near.edge - 2.5) / 0.5))".to_string(),
      "scale * 1.3 * select(params.rules.w, TUNDRA_GRASS_HEIGHT, tundra)".to_string(),
      "0.7 + 1.3 * size_roll * select(0.6, 1.0, fern)".to_string(),
      "dryness + 2.0 * f32(sector + 1)".to_string(),
    ] {
      assert!(
        wgsl.contains(&line),
        "the grass generator no longer has {line}"
      );
    }

    assert!(include_str!("../shaders/grass_instances.wgsl")
      .contains("let lean_code = floor(in.instance_dryness * 0.5);"));
  }
}
