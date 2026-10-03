use vista_types::{BiomeKind, FloraOptions, FloraRule};

use crate::render::terrain_mesh::full_detail_height;
use crate::render::tree_models::{TreeSpecies, SPECIES_COUNT};
use crate::terrain::biomes::SurfaceSample;
use crate::terrain::heightmap::HeightMap;

/// Clamp flora instances to a device or implementation limit.
pub fn clamp_flora_instances(options: &FloraOptions, device_limit: u32) -> u32 {
  if !options.enabled {
    return 0;
  }

  options.max_instances.min(device_limit)
}

/// One GPU-ready grass tuft instance.
///
/// Layout must stay in sync with the per-instance vertex attributes declared
/// in `grass_instances.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FloraInstance {
  /// World position of the base of the plant, in terrain metres.
  pub position: [f32; 3],
  /// Tuft scale in metres.
  pub scale: f32,
  /// A deterministic 0 to 1 tint variation used to vary colour.
  pub tint: f32,
  /// Climate dryness from 0 (lush green) to 1 (straw).
  pub dryness: f32,
  /// [`GRASS_STYLE_TUFT`], [`GRASS_STYLE_REED`], [`GRASS_STYLE_FERN`] or
  /// [`GRASS_STYLE_UNDERGROWTH`]. Streamed tufts carry their rank
  /// relative to `p` in the fraction, for thinning with distance; drawn
  /// tufts carry their extra width there, as `(width - 1) / 4`.
  pub style: f32,
}

/// A grass tuft.
pub const GRASS_STYLE_TUFT: f32 = 0.0;
/// A clump of reeds, 1.4 to 2.2 m tall, beside still or slow water.
pub const GRASS_STYLE_REED: f32 = 1.0;
/// A fern: 5 to 7 curved fronds, 0.4 to 0.9 m, under canopy in temperate
/// and wet ground.
pub const GRASS_STYLE_FERN: f32 = 2.0;
/// Undergrowth: a low leafy clump, 0.3 to 0.6 m, under any canopy.
pub const GRASS_STYLE_UNDERGROWTH: f32 = 3.0;

/// One base-geometry vertex shared by every grass tuft instance.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FloraVertex {
  /// Local offset in billboard space: x across the billboard, y from base
  /// (0) to top (1).
  pub local_offset: [f32; 2],
  /// Texture-free coordinate used by the fragment shader to carve out a
  /// trunk and canopy silhouette.
  pub uv: [f32; 2],
}

/// Bits of [`TreeInstance::species`] that hold the [`TreeSpecies`] index.
pub const TREE_SPECIES_BITS: u32 = 0xff;
/// [`TreeInstance::species`] flag: the tree is grounded on the drawn
/// terrain (see [`grounded_base`]).
pub const TREE_GROUNDED: u32 = 1 << 8;
/// [`TreeInstance::species`] flag: a stunted tree on an exposed ridge or
/// near the tree line, for wind-shaped crowns.
pub const TREE_STUNTED: u32 = 1 << 9;
/// The lowest bit of a tree's variant, 0 to 3 in bits 10 and 11: which
/// of its species' grown shapes it takes. Hand-placed trees leave it 0.
pub const TREE_VARIANT_SHIFT: u32 = 10;
/// The lowest bit of a tree's age class (`tree_growth::Age`: mature,
/// young, old, krummholz), 0 to 3 in bits 12 and 13. Hand-placed trees
/// leave it 0: mature.
pub const TREE_AGE_SHIFT: u32 = 12;
/// The lowest bit of a tree's lean, in bits 14 to 19: the direction it
/// leans (one of 16, clockwise from +z) in bits 14 to 17, and how far (0
/// to 3, up to 6 degrees) in bits 18 and 19. Hand-placed trees stand
/// upright.
pub const TREE_LEAN_SHIFT: u32 = 14;
/// [`TreeInstance::species`] flag: a procedural tree on the lattice (see
/// `render/lattice.rs`), which the cull pass thins with distance by its
/// rank.
pub const TREE_LATTICE: u32 = 1 << 20;
/// The lowest bit of a lattice tree's rank relative to `p`, 0 to
/// `vegetation::RANK_LEVELS` in bits 21 to 31.
pub const TREE_RANK_SHIFT: u32 = 21;

/// One GPU-ready tree instance (32 bytes).
///
/// Layout must stay in sync with `TreeInstance` in `tree_cull.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TreeInstance {
  /// World position of the base of the trunk, in terrain metres.
  pub position: [f32; 3],
  /// Uniform scale applied to the species model.
  pub scale: f32,
  /// Rotation around the vertical axis in radians.
  pub rotation: f32,
  /// Colour variation from 0 to 1.
  pub tint: f32,
  /// [`TreeSpecies`] index in the low 8 bits, then the
  /// [`TREE_GROUNDED`] and [`TREE_STUNTED`] flags, and for lattice trees
  /// [`TREE_LATTICE`] and the rank above [`TREE_RANK_SHIFT`].
  pub species: u32,
  /// Colour dryness from 0 (lush) to 1 (dry), from the local climate.
  pub dryness: f32,
}

impl TreeInstance {
  /// The [`TreeSpecies`] index, without the flags.
  pub fn species_index(&self) -> u32 {
    self.species & TREE_SPECIES_BITS
  }

  /// Whether this tree is grounded on the drawn terrain.
  pub fn grounded(&self) -> bool {
    self.species & TREE_GROUNDED != 0
  }

  /// Whether this tree is stunted by exposure or the tree line.
  pub fn stunted(&self) -> bool {
    self.species & TREE_STUNTED != 0
  }

  /// Its variant, 0 to 3.
  pub fn variant(&self) -> u32 {
    self.species >> TREE_VARIANT_SHIFT & 3
  }

  /// Its age class (`tree_growth::Age`), 0 to 3.
  pub fn age(&self) -> u32 {
    self.species >> TREE_AGE_SHIFT & 3
  }

  /// Its lean: direction sector (0 to 15) and strength (0 to 3).
  pub fn lean(&self) -> (u32, u32) {
    let lean = self.species >> TREE_LEAN_SHIFT;
    (lean & 15, lean >> 4 & 3)
  }
}

/// Numbers per tree in the packed array `setTreeInstances` sends: x, y,
/// z, scale, rotation, tint, species index, dryness and a ground flag
/// (0 or 1).
pub const TREE_PLACEMENT_FLOATS: usize = 9;

/// Unpack host tree placements, [`TREE_PLACEMENT_FLOATS`] numbers per
/// tree, into instances. A ground flag of 1 sets [`TREE_GROUNDED`], so
/// the GPU stands the tree on the drawn terrain and ignores its y. Values
/// are range-checked later by the engine; this only rejects a malformed
/// layout, species or flag.
pub fn unpack_tree_placements(packed: &[f32]) -> Result<Vec<TreeInstance>, String> {
  if !packed.len().is_multiple_of(TREE_PLACEMENT_FLOATS) {
    return Err(format!(
      "tree instances must hold {TREE_PLACEMENT_FLOATS} numbers per tree, but {} numbers were given.",
      packed.len()
    ));
  }

  let count = packed.len() / TREE_PLACEMENT_FLOATS;

  if count > crate::engine::MAX_CUSTOM_TREES {
    return Err(format!(
      "setTreeInstances accepts at most {} trees, but {count} were given.",
      crate::engine::MAX_CUSTOM_TREES
    ));
  }

  let mut trees = Vec::with_capacity(count);

  for (index, tree) in packed.chunks_exact(TREE_PLACEMENT_FLOATS).enumerate() {
    let (species, ground) = (tree[6], tree[8]);

    if !(0.0..TreeSpecies::ALL.len() as f32).contains(&species)
      || species.fract() != 0.0
      || (ground != 0.0 && ground != 1.0)
    {
      return Err(format!(
        "tree {index} must have a species from 0 to 7 and a ground flag of 0 or 1."
      ));
    }

    let ground = if ground == 1.0 { TREE_GROUNDED } else { 0 };
    trees.push(TreeInstance {
      position: [tree[0], tree[1], tree[2]],
      scale: tree[3],
      rotation: tree[4],
      tint: tree[5],
      species: species as u32 | ground,
      dryness: tree[7],
    });
  }

  Ok(trees)
}

/// The height a tree stands at on the terrain mesh centred on `centre`
/// (samples): the lowest drawn ground at the trunk and a root radius out
/// along ±x and ±z, less 5 % of that radius, so the downhill side meets
/// the ground and the uphill side is buried slightly, as real root flares
/// are. Trees stay upright. The GPU grounds trees the same way each time
/// the drawn mesh recentres (`grounded_base` in `ground.wgsl`).
pub fn grounded_base(map: &HeightMap, centre: (f32, f32), x: f32, z: f32, root: f32) -> f32 {
  let ground = |dx: f32, dz: f32| {
    crate::render::terrain_mesh::mesh_surface_height(map, centre, x + dx, z + dz)
  };
  let lowest = ground(0.0, 0.0)
    .min(ground(root, 0.0))
    .min(ground(-root, 0.0))
    .min(ground(0.0, root))
    .min(ground(0.0, -root));
  lowest - 0.05 * root
}

/// [`grounded_base`] on the ground at full detail everywhere, not the
/// mesh as drawn around a camera: where a tree stands wherever the
/// camera is, for exporting trees.
pub fn grounded_full_detail(map: &HeightMap, x: f32, z: f32, root: f32) -> f32 {
  let ground = |dx: f32, dz: f32| {
    let (sx, sz) = crate::render::terrain_mesh::world_to_sample_coordinates(map, x + dx, z + dz);
    full_detail_height(map, sx, sz)
  };
  let lowest = ground(0.0, 0.0)
    .min(ground(root, 0.0))
    .min(ground(-root, 0.0))
    .min(ground(0.0, root))
    .min(ground(0.0, -root));
  lowest - 0.05 * root
}

/// The species that grow in `biome` at a temperature unit, and each
/// one's share of its trees.
pub fn species_table(biome: BiomeKind, temperature: f32) -> &'static [(TreeSpecies, f32)] {
  use TreeSpecies::*;

  let cold = temperature < 0.38;
  let warm = temperature > 0.55;

  match biome {
    BiomeKind::GrassyMeadows => &[(Oak, 0.65), (Shrub, 0.35)],
    BiomeKind::OuterThicket if cold => &[(Shrub, 0.5), (Spruce, 0.25), (Pine, 0.25)],
    BiomeKind::OuterThicket => &[(Shrub, 0.55), (Oak, 0.35), (Pine, 0.1)],
    BiomeKind::OuterForest if cold => &[(Pine, 0.5), (Spruce, 0.4), (Shrub, 0.1)],
    BiomeKind::OuterForest => &[(Oak, 0.55), (Pine, 0.3), (Shrub, 0.15)],
    BiomeKind::InnerForest if cold => &[(Spruce, 0.65), (Pine, 0.35)],
    BiomeKind::InnerForest => &[(Oak, 0.6), (Pine, 0.25), (Spruce, 0.15)],
    BiomeKind::MountainFoothills => &[(Pine, 0.5), (Spruce, 0.4), (Shrub, 0.1)],
    BiomeKind::MountainProper => &[(Spruce, 0.8), (Pine, 0.2)],
    BiomeKind::OuterVolcanic => &[(Pine, 0.7), (Shrub, 0.3)],
    BiomeKind::SavannahExpanse => &[(Acacia, 0.8), (Shrub, 0.2)],
    BiomeKind::CoastalBeach if warm => &[(Palm, 1.0)],
    BiomeKind::CoastalBeach => &[(Pine, 0.6), (Shrub, 0.4)],
    BiomeKind::CoastalRocky if warm => &[(Palm, 0.4), (Shrub, 0.6)],
    BiomeKind::CoastalRocky => &[(Pine, 0.6), (Shrub, 0.4)],
    BiomeKind::OuterJungle => &[(Jungle, 0.5), (Palm, 0.3), (Shrub, 0.2)],
    BiomeKind::InnerJungle => &[(Jungle, 0.85), (Palm, 0.15)],
    BiomeKind::SwampWetlands => &[(Cypress, 0.8), (Shrub, 0.2)],
    BiomeKind::AlpineTransition => &[(Shrub, 0.7), (Spruce, 0.2), (Pine, 0.1)],
    // Only dwarf shrubs survive on the tundra; nothing grows on the ice.
    BiomeKind::IceArctic => &[(Shrub, 1.0)],
    BiomeKind::CalderaVolcanic
    | BiomeKind::Ocean
    | BiomeKind::LowerSnowyPeaks
    | BiomeKind::UpperSnowyPeaks => &[],
  }
}

/// Pick a species for a tree growing in `biome` by its share of the
/// biome's trees alone, or `None` when this biome stays treeless.
pub fn choose_species(biome: BiomeKind, temperature: f32, roll: f32) -> Option<TreeSpecies> {
  let table = species_table(biome, temperature);
  let mut remaining = roll.clamp(0.0, 0.9999);

  for (species, weight) in table {
    if remaining < *weight {
      return Some(*species);
    }

    remaining -= weight;
  }

  table.last().map(|(species, _)| *species)
}

/// Pick a species using a host-supplied rule instead of the built-in mix.
pub fn choose_species_by_rule(rule: &FloraRule, roll: f32) -> Option<TreeSpecies> {
  let total: f32 = rule
    .species
    .iter()
    .map(|choice| choice.weight.max(0.0))
    .sum();

  if total <= 0.0 {
    return None;
  }

  let mut remaining = roll.clamp(0.0, 0.9999) * total;

  for choice in &rule.species {
    let weight = choice.weight.max(0.0);

    if remaining < weight {
      return Some(TreeSpecies::ALL[choice.species.index()]);
    }

    remaining -= weight;
  }

  None
}

/// Where a species grows: its ecological niche. Suitability for a site is
/// the product of factors built from these (see [`build_tree_instances`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeciesNiche {
  /// Annual mean temperature in °C: where it stops, grows best, and stops
  /// again.
  pub celsius: (f32, f32, f32),
  /// Effective moisture (see [`effective_moisture`]; about 0 to 1.5 by
  /// water): least, optimum and most. Most trees grow best on moist
  /// ground and only fail where it is waterlogged, so the response stays
  /// at its best from the optimum to halfway to the most (see
  /// [`moisture_response`]).
  pub moisture: (f32, f32, f32),
  /// Steepest ground it grows on, in degrees.
  pub max_slope_degrees: f32,
  /// How bankside water suits it, from -1 (dry ground) to 1 (wet
  /// ground).
  pub water_affinity: f32,
  /// How well it grows in shade, 0 to 1: on shaded slopes and as a
  /// seedling under its parents.
  pub shade_tolerance: f32,
  /// How well it stands wind on exposed ground, 0 to 1.
  pub exposure_tolerance: f32,
  /// Whether it grows on beach sand.
  pub beach_ok: bool,
  /// How far from a parent its seedlings take root, in metres.
  pub cluster_radius_metres: f32,
}

#[allow(clippy::too_many_arguments)]
const fn niche(
  celsius: (f32, f32, f32),
  moisture: (f32, f32, f32),
  max_slope_degrees: f32,
  water_affinity: f32,
  shade_tolerance: f32,
  exposure_tolerance: f32,
  beach_ok: bool,
  cluster_radius_metres: f32,
) -> SpeciesNiche {
  SpeciesNiche {
    celsius,
    moisture,
    max_slope_degrees,
    water_affinity,
    shade_tolerance,
    exposure_tolerance,
    beach_ok,
    cluster_radius_metres,
  }
}

/// One niche per [`TreeSpecies`], in its order. Temperatures follow this
/// engine's climate, whose warm biomes begin at about 12 °C.
pub const SPECIES_NICHES: [SpeciesNiche; SPECIES_COUNT] = [
  // Oak: temperate, deep moist soils.
  niche(
    (-2.0, 11.0, 28.0),
    (0.3, 0.8, 1.7),
    38.0,
    0.2,
    0.5,
    0.4,
    false,
    12.0,
  ),
  // Pine: hardy, dry and poor ground, windy ridges.
  niche(
    (-4.0, 8.0, 30.0),
    (0.15, 0.5, 1.5),
    45.0,
    -0.2,
    0.2,
    0.7,
    false,
    10.0,
  ),
  // Spruce: cold and moist, shade-bearing.
  niche(
    (-8.0, 2.0, 20.0),
    (0.25, 0.85, 1.8),
    45.0,
    0.2,
    0.8,
    0.6,
    false,
    7.0,
  ),
  // Palm: warm coasts and sand.
  niche(
    (8.0, 24.0, 40.0),
    (0.2, 0.75, 1.8),
    25.0,
    0.3,
    0.3,
    0.5,
    true,
    6.0,
  ),
  // Rainforest emergent: hot and wet, sheltered.
  niche(
    (8.0, 24.0, 40.0),
    (0.4, 1.0, 2.0),
    40.0,
    0.4,
    0.7,
    0.2,
    false,
    15.0,
  ),
  // Bald cypress: standing in wet ground.
  niche(
    (2.0, 16.0, 34.0),
    (0.5, 1.1, 2.2),
    15.0,
    1.0,
    0.5,
    0.3,
    false,
    8.0,
  ),
  // Acacia: hot, dry savannah, not wet ground.
  niche(
    (8.0, 24.0, 40.0),
    (0.0, 0.3, 0.9),
    30.0,
    -0.6,
    0.1,
    0.6,
    false,
    15.0,
  ),
  // Shrub: everywhere trees struggle, up to steep scree.
  niche(
    (-4.0, 10.0, 40.0),
    (0.1, 0.5, 1.6),
    50.0,
    0.1,
    0.6,
    0.9,
    false,
    3.0,
  ),
];

/// Height of breast height, where a trunk's radius is measured, in
/// metres.
pub const TRUNK_BREAST_HEIGHT: f32 = 1.3;

/// Trunk radius at breast height of a mature tree of each species at
/// scale 1, in metres, in [`TreeSpecies`] order: the one source for both
/// the grown trunks (`render/tree_growth.rs`) and the roots grounding
/// reaches over ([`species_root_radius`]). A rainforest emergent's counts
/// its buttresses, and a shrub's the spread of its stems.
pub const TRUNK_RADII: [f32; SPECIES_COUNT] = [0.48, 0.42, 0.38, 0.21, 0.85, 0.62, 0.25, 0.6];

/// Roots spread over this many trunk radii.
pub const ROOT_RADII: f32 = 2.5;

/// The trunk radius of a species at breast height (see [`TRUNK_RADII`]).
pub fn species_trunk_radius(species: TreeSpecies) -> f32 {
  TRUNK_RADII[species as usize]
}

/// How far a species' roots reach, in metres at scale 1: trees sink to
/// the lowest ground within it ([`grounded_base`]).
pub fn species_root_radius(species: TreeSpecies) -> f32 {
  species_trunk_radius(species) * ROOT_RADII
}

/// The niche of a species.
pub fn species_niche(species: TreeSpecies) -> &'static SpeciesNiche {
  &SPECIES_NICHES[species as usize]
}

/// The cover texture's gain: forest cover, suitability and groves
/// combine into each texel's share of the full density, times
/// this. It calibrates the lattice to the tree counts of the grid it
/// replaced (see `the_lattice_keeps_the_old_counts` in `engine.rs`).
const COVER_GAIN: f32 = 5.2;

/// Wavelength of the grove noise, in metres.
const GROVE_METRES: f32 = 180.0;

/// Young trees each tree seeds nearby where it grows at its best: three
/// seedlings with a chance of 0.45 each.
const SEEDLINGS: f32 = 1.35;

/// Trees keep this far clear of a drawn channel's bank, and of a plunge
/// pool's rim, in metres.
pub const CHANNEL_CLEARANCE: f32 = 1.5;
/// See [`CHANNEL_CLEARANCE`].
pub const POOL_CLEARANCE: f32 = 1.0;

/// Neighbourhood, in metres, whose mean height sets topographic exposure.
const EXPOSURE_METRES: f32 = 300.0;

/// What tree placement knows beyond the heights and the surface.
#[derive(Clone, Copy, Debug, Default)]
pub struct Placement<'a> {
  /// Rivers, lakes and falls, when there are any.
  pub rivers: Option<&'a crate::render::water::RiverNetwork>,
  /// Unit direction (world x, z) away from the sun's mean position: the
  /// way shaded slopes face. `[0, 0]` means north (-z).
  pub poleward: [f32; 2],
  /// Upstream area for when the rivers carry no hydrology (rivers off):
  /// see `DrainageArea::d8`. The rivers' own drainage wins when present.
  pub drainage: Option<&'a crate::terrain::drainage::DrainageArea>,
}

/// The unit direction away from the sun's mean position for a sun at
/// `azimuth_degrees`: north (-z) when the sun crosses the southern sky,
/// south when it crosses the northern sky, and north when it is due east
/// or west.
pub fn poleward_of_sun(azimuth_degrees: f32) -> [f32; 2] {
  let equatorward = crate::maths::sun_direction_vector(azimuth_degrees, 0.0)[2];

  if equatorward < -0.05 {
    [0.0, 1.0]
  } else {
    [0.0, -1.0]
  }
}

/// Effective moisture for a species at a site: the surface moisture
/// (which already holds the bankside `0.45 x riparian`), plus up to 0.25
/// for the water gathering from `drainage` samples upstream, the shaded
/// slope's `aspect` term, and the species' own liking for bankside water,
/// `0.35 x water_affinity x riparian`. Distance to water is not added
/// again: the riparian value carries it.
pub fn effective_moisture(
  moisture: f32,
  drainage: f32,
  aspect: f32,
  water_affinity: f32,
  riparian: f32,
) -> f32 {
  let gathered = 0.25 * (drainage.max(1.0).ln() / 12.0).clamp(0.0, 1.0);
  moisture + gathered + aspect + 0.35 * water_affinity * riparian
}

/// A triangular response: 0 at `low` and `high`, 1 at `best`.
fn triangle(value: f32, (low, best, high): (f32, f32, f32)) -> f32 {
  if value <= low || value >= high {
    0.0
  } else if value <= best {
    (value - low) / (best - low).max(1e-6)
  } else {
    (high - value) / (high - best).max(1e-6)
  }
}

/// The moisture response: rising from `low` to 1 at `best`, level to
/// halfway between `best` and `high`, then falling to 0 at `high`. Extra
/// water helps a tree short of it and does not harm one with enough until
/// the ground turns waterlogged, so a shaded slope or a stream never
/// thins a forest that is already watered.
fn moisture_response(value: f32, (low, best, high): (f32, f32, f32)) -> f32 {
  let wet = 0.5 * (best + high);
  triangle(value.min(best).max(value - (wet - best)), (low, best, high))
}

/// One stretch of drawn water trees keep clear of: a segment of a
/// stream's centreline, or a plunge pool (a segment of no length).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Water {
  pub(crate) a: [f32; 2],
  pub(crate) b: [f32; 2],
  /// Half width at `a` and at `b`.
  pub(crate) half: [f32; 2],
  pub(crate) clearance: f32,
  /// The stream's bed stones along it: their median size in metres and
  /// how many of the stone lattice's points hold one (see
  /// `boulders::stone_bed`); none for pools.
  pub(crate) stones: [f32; 2],
  /// How densely riparian plants line it (see `water::riparian_band`);
  /// none for pools.
  pub(crate) band: f32,
}

impl Water {
  /// How far `(x, z)` is inside this water's clearance, in metres
  /// (positive when a tree there would stand too close).
  pub(crate) fn intrusion(&self, x: f32, z: f32) -> f32 {
    let (dx, dz) = (self.b[0] - self.a[0], self.b[1] - self.a[1]);
    let length = dx * dx + dz * dz;
    let t = if length > 0.0 {
      (((x - self.a[0]) * dx + (z - self.a[1]) * dz) / length).clamp(0.0, 1.0)
    } else {
      0.0
    };
    let (px, pz) = (self.a[0] + dx * t - x, self.a[1] + dz * t - z);
    let half = self.half[0] + (self.half[1] - self.half[0]) * t;
    half + self.clearance - (px * px + pz * pz).sqrt()
  }
}

/// Topographic exposure: each sample's height above the mean of the
/// ground within [`EXPOSURE_METRES`], from a box filter on a coarse grid.
struct Exposure {
  /// Samples per coarse cell.
  step: u32,
  width: u32,
  mean: Vec<f32>,
}

impl Exposure {
  fn build(map: &HeightMap) -> Self {
    let (width, height) = (map.metadata.width, map.metadata.height);
    let metres = map.metadata.metres_per_sample.max(0.001);
    let step = ((30.0 / metres).round() as u32).max(1);
    let (cw, ch) = ((width - 1) / step + 1, (height - 1) / step + 1);
    let radius = ((EXPOSURE_METRES * 0.5 / (step as f32 * metres)).round() as usize).max(1);
    let coarse: Vec<f32> = (0..cw * ch)
      .map(|cell| map.heights[((cell / cw) * step * width + (cell % cw) * step) as usize])
      .collect();
    // One box pass each way: the mean over the square around each cell,
    // with the map's edge repeated beyond it.
    let box_pass = crate::terrain::glaciers::box_pass;
    let (columns, lines) = (cw as usize, ch as usize);
    let mut across = vec![0.0; coarse.len()];
    let mut mean = vec![0.0; coarse.len()];
    box_pass(&coarse, &mut across, columns, lines, radius, true);
    box_pass(&across, &mut mean, columns, lines, radius, false);

    Self {
      step,
      width: cw,
      mean,
    }
  }

  fn at(&self, map: &HeightMap, x: u32, y: u32) -> f32 {
    let (cx, cy) = (
      ((x + self.step / 2) / self.step).min(self.width - 1),
      ((y + self.step / 2) / self.step).min((self.mean.len() as u32 / self.width) - 1),
    );
    map.heights[(y * map.metadata.width + x) as usize] - self.mean[(cy * self.width + cx) as usize]
  }
}

/// Everything placement reads, built once per call.
struct Fields<'a> {
  map: &'a HeightMap,
  surface: &'a [SurfaceSample],
  options: &'a FloraOptions,
  metres: f32,
  half: [f32; 2],
  water_line: f32,
  waters: Option<crate::water_sounds::BucketGrid<Water>>,
  /// Samples a river's bed or bars cover, below the bank top.
  bed: Vec<bool>,
  riparian: &'a [u8],
  /// Upstream area from the rivers' hydrology, or the D8 fallback.
  drainage: Option<&'a crate::terrain::drainage::DrainageArea>,
  exposure: Exposure,
  poleward: [f32; 2],
}

/// A place a tree might stand, with what its species' suitability needs.
#[derive(Clone, Copy, Debug)]
struct Site {
  sample: SurfaceSample,
  elevation: f32,
  slope_degrees: f32,
  /// Height above the mean of the neighbourhood, in metres.
  exposure: f32,
  /// Moisture the shaded side of a slope keeps (0 to 0.12).
  aspect: f32,
  drainage: f32,
  riparian: f32,
}

impl<'a> Fields<'a> {
  fn new(
    map: &'a HeightMap,
    surface: &'a [SurfaceSample],
    options: &'a FloraOptions,
    placement: &Placement<'a>,
  ) -> Self {
    let metres = map.metadata.metres_per_sample.max(0.001);
    let half = [
      (map.metadata.width as f32 - 1.0) * metres * 0.5,
      (map.metadata.height as f32 - 1.0) * metres * 0.5,
    ];
    let rivers = placement.rivers;
    let waters = rivers.map(|rivers| waters(map, rivers, half));
    let mut bed = Vec::new();

    if let Some(rivers) = rivers.filter(|rivers| !rivers.bed.is_empty()) {
      bed = vec![false; map.heights.len()];

      for (index, _) in &rivers.bed {
        if let Some(slot) = bed.get_mut(*index as usize) {
          *slot = true;
        }
      }
    }

    let poleward = if placement.poleward == [0.0, 0.0] {
      [0.0, -1.0]
    } else {
      placement.poleward
    };

    Self {
      map,
      surface,
      options,
      metres,
      half,
      water_line: map.metadata.sea_level_metres + 0.6,
      waters,
      bed,
      riparian: rivers.map_or(&[][..], |rivers| rivers.riparian.as_slice()),
      drainage: rivers
        .map(|rivers| &rivers.drainage)
        .filter(|area| !area.cells.is_empty())
        .or(placement.drainage),
      exposure: Exposure::build(map),
      poleward,
    }
  }

  fn world(&self, x: f32, z: f32) -> [f32; 2] {
    [
      x * self.metres - self.half[0],
      z * self.metres - self.half[1],
    ]
  }

  /// The site at sample coordinates `(x, z)`, or `None` where no tree may
  /// grow at all. Rules read the sample under the point.
  fn site(&self, x: f32, z: f32) -> Option<Site> {
    use crate::terrain::biomes::{MAT_GRAVEL, MAT_ROCK, MAT_SNOW};

    let map = self.map;
    let (width, height) = (map.metadata.width, map.metadata.height);

    // Nothing grows off the map, on the skirt.
    if x < 0.0 || z < 0.0 || x > (width - 1) as f32 || z > (height - 1) as f32 {
      return None;
    }

    let (sx, sz) = (x.round() as u32, z.round() as u32);
    let index = (sz * width + sx) as usize;
    let sample = self.surface[index];
    let elevation = map.heights[index];

    // Weights are out of 255: gravel over 0.4, rock over 0.6, snow and
    // lasting snow over 0.5, and heat over 0.2 keep trees out.
    if map.no_data[index]
      || elevation <= self.water_line
      || sample.river > 0
      || sample.is_glacier()
      || sample.materials[MAT_GRAVEL] > 102
      || sample.materials[MAT_ROCK] > 153
      || sample.materials[MAT_SNOW] > 127
      || sample.permanent_snow > 127
      || sample.heat > 51
      || sample.celsius() < -1.0
      || elevation >= self.options.tree_line_metres
      || matches!(
        sample.biome_kind(),
        BiomeKind::LowerSnowyPeaks | BiomeKind::UpperSnowyPeaks
      )
      || self.bed.get(index).copied().unwrap_or(false)
    {
      return None;
    }

    if let Some(waters) = &self.waters {
      let [wx, wz] = self.world(x, z);

      let mut inside = false;
      waters.for_each_near(wx, wz, |water| inside |= water.intrusion(wx, wz) > 0.0);

      if inside {
        return None;
      }
    }

    // The fine per-sample normal, from the samples either side, as the
    // terrain's own normals are. Its horizontal part points downhill and
    // is as long as the slope's sine.
    let at = |dx: i32, dz: i32| {
      let nx = (sx as i32 + dx).clamp(0, width as i32 - 1) as u32;
      let nz = (sz as i32 + dz).clamp(0, height as i32 - 1) as u32;
      map.heights[(nz * width + nx) as usize]
    };
    let normal = crate::maths::normalise([
      (at(-1, 0) - at(1, 0)) / (2.0 * self.metres),
      1.0,
      (at(0, -1) - at(0, 1)) / (2.0 * self.metres),
    ]);
    let slope_degrees = normal[1].clamp(-1.0, 1.0).acos().to_degrees();
    let shaded = normal[0] * self.poleward[0] + normal[2] * self.poleward[1];

    Some(Site {
      sample,
      elevation,
      slope_degrees,
      exposure: self.exposure.at(map, sx, sz),
      aspect: 0.12 * (shaded * 2.0).clamp(0.0, 1.0),
      drainage: self.drainage.map_or(1.0, |area| area.at(sx, sz)),
      riparian: f32::from(self.riparian.get(index).copied().unwrap_or(0)) / 255.0,
    })
  }
}

/// Channels (every drawn stream, loops included) and plunge pools, sorted
/// into a grid whose cells hold every segment that could reach a tree in
/// them.
pub(crate) fn waters(
  map: &HeightMap,
  rivers: &crate::render::water::RiverNetwork,
  half: [f32; 2],
) -> crate::water_sounds::BucketGrid<Water> {
  let metres = map.metadata.metres_per_sample.max(0.001);
  let mut items = Vec::new();

  for (index, run) in rivers.channels.iter().enumerate() {
    let stones = rivers
      .stones
      .get(index)
      .filter(|stones| stones.len() == run.len());
    let bands = rivers
      .bands
      .get(index)
      .filter(|bands| bands.len() == run.len());

    for (k, pair) in run.windows(2).enumerate() {
      items.push(Water {
        a: [pair[0][0], pair[0][1]],
        b: [pair[1][0], pair[1][1]],
        half: [pair[0][2], pair[1][2]],
        clearance: CHANNEL_CLEARANCE,
        stones: stones.map_or([0.0; 2], |stones| {
          [0, 1].map(|i| 0.5 * (stones[k][i] + stones[k + 1][i]))
        }),
        band: bands.map_or(0.0, |bands| 0.5 * (bands[k] + bands[k + 1])),
      });
    }
  }

  let pool = |foot: [f32; 2], radius: f32| {
    let at = [foot[0] * metres - half[0], foot[1] * metres - half[1]];
    Water {
      a: at,
      b: at,
      half: [radius; 2],
      clearance: POOL_CLEARANCE,
      stones: [0.0; 2],
      band: 0.0,
    }
  };

  for fall in rivers.falls.iter().filter(|fall| !fall.trickle) {
    items.push(pool(fall.foot, fall.pool_radius));

    for step in &fall.steps {
      items.push(pool(step.foot, step.pool_radius));
    }
  }

  // A cell holds each item by its middle, so it must span half the
  // longest item and its reach beyond it.
  let reach = items
    .iter()
    .map(|w| {
      let length = ((w.b[0] - w.a[0]).powi(2) + (w.b[1] - w.a[1]).powi(2)).sqrt();
      0.5 * length + w.half[0].max(w.half[1]) + w.clearance
    })
    .fold(0.0f32, f32::max);
  // Cells are at least 64 m, the bins the GPU generators read (see
  // `vegetation::channel_bins`).
  let cell = reach.max(64.0);
  crate::water_sounds::BucketGrid::build(half, cell, items, |w| {
    [(w.a[0] + w.b[0]) * 0.5, (w.a[1] + w.b[1]) * 0.5]
  })
}

/// The slope response: 1 up to [`FULL_SLOPE`] of the species' maximum
/// slope, falling to 0 at the maximum. Mountain forests stand on 35 to 45
/// degree slopes wherever there is soil; rock is excluded separately.
pub fn slope_response(slope_degrees: f32, max_slope_degrees: f32) -> f32 {
  ((max_slope_degrees - slope_degrees) / ((1.0 - FULL_SLOPE) * max_slope_degrees)).clamp(0.0, 1.0)
}

/// Fraction of a species' maximum slope up to which slope does not thin
/// it.
const FULL_SLOPE: f32 = 0.75;

/// How well `niche` suits `site`, 0 to 1: temperature, effective
/// moisture, slope and exposure. Sand keeps out all but beach species,
/// and nothing grows steeper than its maximum slope.
fn suitability(site: &Site, niche: &SpeciesNiche) -> f32 {
  use crate::terrain::biomes::MAT_SAND;

  if (site.sample.weight(MAT_SAND) > 0.5 && !niche.beach_ok)
    || site.slope_degrees >= niche.max_slope_degrees
  {
    return 0.0;
  }

  let moisture = effective_moisture(
    site.sample.moisture_unit(),
    site.drainage,
    site.aspect,
    niche.water_affinity,
    site.riparian,
  )
  .clamp(0.0, 2.0);
  let slope = slope_response(site.slope_degrees, niche.max_slope_degrees);
  let exposure = 1.0 - (1.0 - niche.exposure_tolerance) * (site.exposure / 60.0).clamp(0.0, 1.0);
  triangle(site.sample.celsius(), niche.celsius)
    * moisture_response(moisture, niche.moisture)
    * slope
    * exposure
}

/// Scatter deterministic tree instances by the land, with no rivers and
/// the sun in the southern sky. See [`build_tree_instances_with`].
pub fn build_tree_instances(
  map: &HeightMap,
  surface: &[SurfaceSample],
  options: &FloraOptions,
  density_scale: f32,
) -> Vec<TreeInstance> {
  build_tree_instances_with(map, surface, options, density_scale, &Placement::default())
}

/// Every procedural tree over the map, placed on the CPU: each lattice
/// point (see `render/lattice.rs`) whose rank is below `p`, where `p`
/// comes from the cover texture ([`bake_cover`]). This is the reference
/// the GPU's far set and streamed tiles add up to; see
/// [`build_lattice_trees`].
pub fn build_tree_instances_with(
  map: &HeightMap,
  surface: &[SurfaceSample],
  options: &FloraOptions,
  density_scale: f32,
  placement: &Placement<'_>,
) -> Vec<TreeInstance> {
  build_lattice_trees(map, surface, options, density_scale, placement, 1.0)
}

/// The lattice trees over the map whose rank relative to `p` is below
/// `keep`, at density `options.density x density_scale` (0 to 4): the
/// far set with `keep = FAR_KEEP`. Hard exclusions (water and the banks
/// of every drawn channel, plunge pools, gravel and bars, sand, rock,
/// snow, glacier, volcanic heat, the snowy peaks, the skirt, and ground
/// steeper than the species allows) apply to all. Each tree's height is
/// the ground as the mesh draws it at full detail; the GPU grounds it on
/// the mesh as drawn each time it recentres. Over the instance cap, the
/// trees of lowest rank are kept, which are spread evenly over the map.
/// The same seed and options always give the same trees.
pub fn build_lattice_trees(
  map: &HeightMap,
  surface: &[SurfaceSample],
  options: &FloraOptions,
  density_scale: f32,
  placement: &Placement<'_>,
  keep: f32,
) -> Vec<TreeInstance> {
  let density = (options.density * density_scale).clamp(0.0, 4.0);

  if !options.enabled || density <= 0.0 || options.max_instances == 0 {
    return Vec::new();
  }

  let ground = ground_with_cover(map, surface, options, placement);
  let bins = crate::render::vegetation::channel_bins(map, placement.rivers);
  let rules = crate::render::vegetation::TreeRules::new(options, density);
  stand_lattice_trees(
    map,
    &ground,
    &bins,
    &rules,
    keep,
    options.max_instances as usize,
  )
}

/// [`crate::render::vegetation::lattice_trees`], each standing on the
/// ground as the mesh draws it at full detail.
pub fn stand_lattice_trees(
  map: &HeightMap,
  ground: &crate::render::vegetation::GroundData,
  bins: &[u32],
  rules: &crate::render::vegetation::TreeRules,
  keep: f32,
  max: usize,
) -> Vec<TreeInstance> {
  let mut trees = crate::render::vegetation::lattice_trees(ground, bins, rules, keep, max);

  for tree in &mut trees {
    let (x, z) = crate::render::terrain_mesh::world_to_sample_coordinates(
      map,
      tree.position[0],
      tree.position[2],
    );
    tree.position[1] = full_detail_height(map, x, z);
  }

  trees
}

/// The ground data the generators read, with the cover texture baked.
pub fn ground_with_cover(
  map: &HeightMap,
  surface: &[SurfaceSample],
  options: &FloraOptions,
  placement: &Placement<'_>,
) -> crate::render::vegetation::GroundData {
  let mut ground = crate::render::vegetation::GroundData::of(map);
  let rivers = placement.rivers;
  let dry = crate::render::water::WetBanks::default();
  ground.set_surface(
    map,
    surface,
    rivers.map_or(&dry, |rivers| &rivers.wet),
    rivers.map_or(&[][..], |rivers| rivers.riparian.as_slice()),
  );
  ground.cover = bake_cover(map, surface, options, placement, ground.stride, None);
  ground
}

/// The most a cover texel's share of the table density reaches: prime
/// land holds up to four times the density the slider names, as old
/// growth gathers where the ground suits it best.
pub const COVER_SHARE_MAX: f32 = 4.0;

/// The cover texture's red channel for a share of the table density:
/// `sqrt(share / COVER_SHARE_MAX)`, which keeps precision on sparse land.
/// A share is `red^2 / 255^2 x COVER_SHARE_MAX`, exact in f32.
pub fn cover_red(share: f32) -> u8 {
  ((share / COVER_SHARE_MAX).clamp(0.0, 1.0).sqrt() * 255.0).round() as u8
}

/// The share of the table density a cover texel's red channel holds.
pub fn cover_share(red: u8) -> f32 {
  let red = f32::from(red);
  red * red * (COVER_SHARE_MAX / (255.0 * 255.0))
}

/// Cover texel alpha: bit 7 is set where trees are stunted (the tree line
/// and exposed ridges), and bits 0 to 6 hold the second species' share
/// out of 127.
pub const COVER_STUNTED: u8 = 0x80;

/// The second species' share in every cover texel: the two species are
/// independent draws by weight, so each species keeps its share of the
/// trees on average.
pub const COVER_SECOND_SHARE: u8 = 64;

/// Bake the cover texture: one texel every `stride` samples, from plan
/// 4's placement rules.
///
/// - r: the share of the table density the land supports here (see
///   [`cover_red`]), so `p = target_density(d) x share / 1100` before
///   the lattice's own clumps: forest cover, the species' suitability
///   (temperature, effective moisture, slope, exposure), the tree line
///   and alpine band, the young trees each tree seeds, groves (a 180 m
///   noise from 0.35 to 1) and any host rule's density, times the
///   density mask's multiplier where one is given (one byte a sample:
///   0 none, 128 no change, 255 twice as dense; see
///   `painted::density_multiplier`);
/// - g and b: the dominant and second species, each drawn by the
///   species' weight (share of the biome times suitability);
/// - a: the second species' share out of 127, with [`COVER_STUNTED`].
///
/// Hard exclusions (water and river beds, gravel, sand, rock, snow,
/// glacier, heat, the snowy peaks, the tree line and the skirt) give 0.
pub fn bake_cover(
  map: &HeightMap,
  surface: &[SurfaceSample],
  options: &FloraOptions,
  placement: &Placement<'_>,
  stride: u32,
  mask: Option<&[u8]>,
) -> Vec<[u8; 4]> {
  let (width, height) = (map.metadata.width, map.metadata.height);
  let stride = stride.max(1);
  let (columns, rows) = (
    width.saturating_sub(1) / stride + 1,
    height.saturating_sub(1) / stride + 1,
  );

  if !options.enabled
    || width < 2
    || height < 2
    || surface.len() != map.heights.len()
    || mask.is_some_and(|mask| mask.len() != map.heights.len())
  {
    return vec![[0; 4]; (columns * rows) as usize];
  }

  let fields = Fields::new(map, surface, options, placement);
  let seed = crate::render::lattice::lattice_seed(options.seed_offset);

  (0..columns * rows)
    .map(|texel| {
      let (sx, sz) = (
        ((texel % columns) * stride).min(width - 1),
        ((texel / columns) * stride).min(height - 1),
      );
      let multiplier = mask.map_or(1.0, |mask| {
        crate::terrain::painted::density_multiplier(mask[(sz * width + sx) as usize])
      });
      cover_texel(&fields, sx, sz, seed, multiplier)
    })
    .collect()
}

/// One texel of [`bake_cover`], at sample `(x, z)`.
fn cover_texel(fields: &Fields<'_>, x: u32, z: u32, seed: u32, multiplier: f32) -> [u8; 4] {
  let Some(site) = fields.site(x as f32, z as f32) else {
    return [0; 4];
  };
  let options = fields.options;
  let biome = site.sample.biome_kind();
  let rule = options
    .species_rules
    .iter()
    .find(|rule| rule.biome == biome);
  // Each species with its weight of the trees here, and how well it
  // grows: suitability times the tree line's and alpine band's density.
  let mut choices = [(TreeSpecies::Oak, 0.0f32, 0.0f32); 8];
  let count = match rule {
    Some(rule) => {
      let mut count = 0;

      for choice in rule.species.iter().filter(|choice| choice.weight > 0.0) {
        let species = TreeSpecies::ALL[choice.species.index()];
        let niche = species_niche(species);
        // A host's rule chooses the species; only the hard limits apply.
        let allowed = site.slope_degrees < niche.max_slope_degrees
          && (niche.beach_ok || site.sample.weight(crate::terrain::biomes::MAT_SAND) <= 0.5);

        if allowed && count < choices.len() {
          choices[count] = (species, choice.weight, 1.0);
          count += 1;
        }
      }

      count
    }
    None => species_choices(&site, &mut choices),
  };
  let choices = &mut choices[..count];

  for (species, _, grows) in choices.iter_mut() {
    *grows *= limits(&site, *species, options).density;
  }

  // Trees per unit of density: each species by its weight and how well
  // it grows, with the parent boost, as each tree seeds up to three young
  // ones nearby, likelier where it grows well and for shade-bearing
  // species.
  let total: f32 = choices
    .iter()
    .map(|(_, weight, _)| weight)
    .sum::<f32>()
    .max(1e-6);
  let expected = choices
    .iter()
    .map(|(species, weight, grows)| {
      let seedlings = SEEDLINGS * grows * (0.4 + 0.6 * species_niche(*species).shade_tolerance);
      weight * grows * (1.0 + seedlings)
    })
    .sum::<f32>()
    / total;
  let [wx, wz] = fields.world(x as f32, z as f32);
  let rule_density = rule.map_or(1.0, |rule| rule.density.clamp(0.0, 4.0));
  let share = COVER_GAIN
    * (site.sample.forest as f32 / 255.0)
    * rule_density
    * grove(options.seed_offset, wx, wz)
    * expected
    * multiplier;
  let share = cover_red(share);

  if share == 0 {
    return [0; 4];
  }

  // Species are drawn by how many of the trees here they make up.
  let hash = crate::render::lattice::pcg3d([x, z, seed ^ 0x51ed_270b]);
  let pick = |roll: f32| {
    let mut remaining = roll * choices.iter().map(|(_, w, g)| w * g).sum::<f32>();

    for (species, weight, grows) in choices.iter() {
      if remaining < weight * grows {
        return *species;
      }

      remaining -= weight * grows;
    }

    choices
      .last()
      .map_or(TreeSpecies::Oak, |(species, _, _)| *species)
  };
  let (first, second) = (
    pick(crate::render::lattice::unit(hash[0])),
    pick(crate::render::lattice::unit(hash[1])),
  );
  let stunted = first != TreeSpecies::Shrub && limits(&site, first, options).stunted;

  [
    share,
    first as u8,
    second as u8,
    COVER_SECOND_SHARE | if stunted { COVER_STUNTED } else { 0 },
  ]
}

/// The tree line, the alpine transition band and exposure, for a species
/// at a site.
struct Limits {
  /// Multiplier on the chance of a tree.
  density: f32,
  /// Whether the tree is stunted.
  stunted: bool,
}

fn limits(site: &Site, species: TreeSpecies, options: &FloraOptions) -> Limits {
  // Full density to 200 m below the tree line, none at it.
  let below_line = options.tree_line_metres - site.elevation;
  let mut density = (below_line / 200.0).clamp(0.0, 1.0);
  // Exposed ridges and the last 150 m below the tree line stunt trees.
  let mut stunting = (site.exposure / 60.0)
    .clamp(0.0, 1.0)
    .max(1.0 - (below_line / 150.0).clamp(0.0, 1.0));

  // Above the trees and below the snow line only shrubs and stunted
  // conifers grow, sparsely, thinning to none at the band's top. The
  // tree line and the band agree: whichever is lower wins.
  if site.sample.biome_kind() == BiomeKind::AlpineTransition {
    let into_band = f32::from(site.sample.band) / 255.0;
    density *= match species {
      TreeSpecies::Shrub | TreeSpecies::Pine | TreeSpecies::Spruce => 0.15 * (1.0 - into_band),
      _ => 0.0,
    };

    if species != TreeSpecies::Shrub {
      stunting = stunting.max(0.6);
    }
  }

  Limits {
    density,
    stunted: stunting > 0.25,
  }
}

/// The species that grow at `site`, written into `out` as (species,
/// choice weight, suitability): among those of its biome with suitability
/// above 0.05, weighted by their share of the biome and their
/// suitability. At forest edges (suitability under 0.3) and on steep
/// ground (over 30 degrees, where a shrub understorey joins every wooded
/// biome) shrubs weigh more, and on shaded slopes the shade-bearing
/// species. Returns how many there are.
fn species_choices(site: &Site, out: &mut [(TreeSpecies, f32, f32); 8]) -> usize {
  let table = species_table(site.sample.biome_kind(), site.sample.temperature_unit());
  let mut count = 0;
  let mut best = 0.0f32;

  for (species, share) in table {
    let suited = suitability(site, species_niche(*species));

    if suited > 0.05 {
      out[count] = (*species, *share, suited);
      count += 1;
      best = best.max(suited);
    }
  }

  // Steep forest ground that its trees cannot hold keeps a shrub
  // understorey, where the shrub's steeper niche allows.
  let steep = crate::maths::smoothstep((site.slope_degrees - 30.0) / 12.0);
  let shrub = TreeSpecies::Shrub;

  if steep > 0.0 && count > 0 && !table.iter().any(|(s, _)| *s == shrub) {
    let suited = suitability(site, species_niche(shrub));

    if suited > 0.05 {
      out[count] = (shrub, 0.35 * steep, suited);
      count += 1;
      best = best.max(suited);
    }
  }

  let edge = (0.3 - best).max(0.0) / 0.3;
  let shaded = site.aspect / 0.12;

  for (species, share, suited) in out[..count].iter_mut() {
    let niche = species_niche(*species);
    let mut weight = *share * *suited * (1.0 + shaded * (niche.shade_tolerance - 0.5));

    if *species == TreeSpecies::Shrub {
      weight *= 1.0 + 2.0 * edge + 2.0 * steep;
    }

    *share = weight.max(0.0);
  }

  count
}

/// Groves: low-frequency noise from 0.35 to 1, so forests gather in
/// stands rather than an even scatter.
fn grove(seed: u64, x: f32, z: f32) -> f32 {
  let noise = crate::maths::value_noise(seed ^ 0x6d0f_27bd, x / GROVE_METRES, z / GROVE_METRES);
  0.35 + 0.65 * crate::maths::smoothstep(0.5 + 0.9 * noise)
}

pub(crate) fn unit_from_hash(value: f32) -> f32 {
  (value + 1.0) * 0.5
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::terrain::biomes::classify_surface;
  use crate::terrain::normals::generate_normals;
  use vista_types::{BiomeOptions, TerrainMetadata};

  fn flat_map(size: u32, elevation: f32) -> HeightMap {
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 4.0,
      sea_level_metres: 0.0,
      max_height_metres: 600.0,
      ..TerrainMetadata::default()
    };

    HeightMap::flat(size, size, elevation, metadata)
  }

  fn forest_surface(map: &HeightMap) -> Vec<SurfaceSample> {
    let options = BiomeOptions {
      moisture_bias: 1.0,
      temperature_bias: -0.3,
      volcanism: 0.0,
      ..BiomeOptions::default()
    };
    classify_surface(map, &generate_normals(map), None, &[], &options)
  }

  fn flora_options() -> FloraOptions {
    FloraOptions {
      enabled: true,
      density: 1.0,
      tree_line_metres: 1_800.0,
      seed_offset: 42,
      max_instances: 10_000,
      ..FloraOptions::default()
    }
  }

  /// A map `size` samples square, `metres` apart, with heights from
  /// world `(x, z)` in metres.
  fn map_from(size: u32, metres: f32, height: impl Fn(f32, f32) -> f32) -> HeightMap {
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: metres,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 0.0, metadata);
    let half = (size as f32 - 1.0) * metres * 0.5;

    for (index, h) in map.heights.iter_mut().enumerate() {
      let (x, z) = ((index as u32 % size) as f32, (index as u32 / size) as f32);
      *h = height(x * metres - half, z * metres - half);
    }

    crate::terrain::heightmap::update_stats(&map.heights, &map.no_data, &mut map.metadata);
    // Lowland and hills, as `flat_map`, not a range of its own.
    map.metadata.max_height_metres = map.metadata.max_height_metres.max(600.0);
    map
  }

  fn sample_of(map: &HeightMap, tree: &TreeInstance) -> (f32, f32) {
    crate::render::terrain_mesh::world_to_sample_coordinates(
      map,
      tree.position[0],
      tree.position[2],
    )
  }

  fn classify(map: &HeightMap, riparian: &[u8], options: &BiomeOptions) -> Vec<SurfaceSample> {
    classify_surface(map, &generate_normals(map), None, riparian, options)
  }

  #[test]
  fn every_tree_stands_on_the_full_detail_ground() {
    // A 30 degree plane on 12 m cells. Trees used to keep the height of
    // the sample nearest their jittered position: up to half a cell
    // off, 6 m x tan 30 degrees = 3.5 m above or below the ground.
    let rise = 30f32.to_radians().tan();
    let map = map_from(96, 12.0, |x, _| 400.0 + x * rise);
    let trees = build_tree_instances(&map, &forest_surface(&map), &flora_options(), 1.0);

    assert!(trees.len() > 100, "{} trees", trees.len());

    for tree in &trees {
      let (x, z) = sample_of(&map, tree);
      let ground = full_detail_height(&map, x, z);
      assert!(
        (tree.position[1] - ground).abs() < 0.05,
        "{} against {ground}",
        tree.position[1]
      );
      assert!(tree.grounded());
    }
  }

  /// A brook 2 m wide looping across a plain on a 12 m grid: loops of
  /// 5 m amplitude and 22 m wavelength, far finer than a sample, drawn
  /// but not in the channel mask.
  fn looping_brook() -> Vec<[f32; 3]> {
    (0..1400)
      .map(|i| {
        let x = -700.0 + i as f32;
        let z = 30.0 + 5.0 * (x / 22.0 * std::f32::consts::TAU).sin() + 3.0 * (x / 9.0).cos();
        [x, z, 1.0]
      })
      .collect()
  }

  fn distance_to(run: &[[f32; 3]], x: f32, z: f32) -> f32 {
    run
      .windows(2)
      .map(|pair| {
        let water = Water {
          a: [pair[0][0], pair[0][1]],
          b: [pair[1][0], pair[1][1]],
          half: [0.0; 2],
          clearance: 0.0,
          stones: [0.0; 2],
          band: 0.0,
        };
        -water.intrusion(x, z)
      })
      .fold(f32::MAX, f32::min)
  }

  #[test]
  fn no_tree_stands_in_a_brook_narrower_than_a_sample_or_its_loops() {
    let map = map_from(128, 12.0, |_, _| 40.0);
    let surface = forest_surface(&map);
    let brook = looping_brook();
    let rivers = crate::render::water::RiverNetwork {
      channels: vec![brook.clone()],
      ..Default::default()
    };
    let placement = Placement {
      rivers: Some(&rivers),
      ..Placement::default()
    };
    let mut options = flora_options();
    options.max_instances = 100_000;
    let trees = build_tree_instances_with(&map, &surface, &options, 1.0, &placement);
    let mut close = 0;

    for tree in &trees {
      let distance = distance_to(&brook, tree.position[0], tree.position[2]);
      assert!(
        distance >= 1.0 + 1.5 - 1e-3,
        "a tree {distance} m from the centreline"
      );
      close += usize::from(distance < 8.0);
    }

    // Trees still reach the bank.
    assert!(close > 10, "{close} trees by the brook");
  }

  #[test]
  fn nothing_grows_on_gravel_bars_or_the_river_bed() {
    use crate::terrain::biomes::{MAT_GRAVEL, MAT_LUSH_GRASS};

    let map = map_from(96, 4.0, |_, _| 40.0);
    let mut surface = forest_surface(&map);

    // Rows 10 to 29 are half gravel; rows 50 to 69 lie in the river bed.
    for z in 10..30 {
      for x in 0..96 {
        let sample = &mut surface[z * 96 + x];
        sample.materials = [0; crate::terrain::biomes::MATERIAL_COUNT];
        sample.materials[MAT_GRAVEL] = 110;
        sample.materials[MAT_LUSH_GRASS] = 145;
      }
    }

    let rivers = crate::render::water::RiverNetwork {
      bed: (50..70)
        .flat_map(|z| (0..96).map(move |x| ((z * 96 + x) as u32, [100, 0, 0, 0])))
        .collect(),
      ..Default::default()
    };
    let placement = Placement {
      rivers: Some(&rivers),
      ..Placement::default()
    };
    let trees = build_tree_instances_with(&map, &surface, &flora_options(), 1.0, &placement);
    let rows: Vec<u32> = trees
      .iter()
      .map(|tree| sample_of(&map, tree).1.round() as u32)
      .collect();

    assert!(rows
      .iter()
      .any(|row| *row < 10 || (30..50).contains(row) || *row >= 70));
    assert!(rows
      .iter()
      .all(|row| !(10..30).contains(row) && !(50..70).contains(row)));
  }

  #[test]
  fn nothing_grows_on_sand_rock_snow_heat_or_above_the_tree_line() {
    use crate::terrain::biomes::{MAT_LUSH_GRASS, MAT_ROCK, MAT_SAND};

    // Five bands of 16 rows each: sand, rock, lasting snow, volcanic heat
    // and ground above the tree line; then ordinary forest.
    let mut map = map_from(96, 4.0, |_, _| 40.0);
    let mut surface = forest_surface(&map);
    let mut options = flora_options();
    options.tree_line_metres = 500.0;

    for z in 0..80usize {
      for x in 0..96usize {
        let index = z * 96 + x;
        let sample = &mut surface[index];
        let band = z / 16;

        if band < 2 {
          sample.materials = [0; crate::terrain::biomes::MATERIAL_COUNT];
          sample.materials[if band == 0 { MAT_SAND } else { MAT_ROCK }] = 170;
          sample.materials[MAT_LUSH_GRASS] = 85;
        } else if band == 2 {
          sample.permanent_snow = 140;
        } else if band == 3 {
          sample.heat = 60;
        } else {
          map.heights[index] = 520.0;
        }
      }
    }

    let trees = build_tree_instances(&map, &surface, &options, 1.0);
    assert!(!trees.is_empty());
    assert!(trees
      .iter()
      .all(|tree| sample_of(&map, tree).1.round() >= 80.0));

    // Palms, and only palms, grow on sand.
    options.species_rules = vec![FloraRule {
      biome: surface[0].biome_kind(),
      species: vec![vista_types::SpeciesWeight {
        species: vista_types::TreeSpeciesKind::Palm,
        weight: 1.0,
      }],
      density: 1.0,
    }];
    let palms = build_tree_instances(&map, &surface, &options, 1.0);
    assert!(palms
      .iter()
      .any(|tree| sample_of(&map, tree).1.round() < 16.0));
  }

  /// A cone rising to 300 m with sides of `degrees`.
  fn cone(degrees: f32) -> HeightMap {
    let rise = degrees.to_radians().tan();
    map_from(160, 8.0, |x, z| {
      (300.0 - (x * x + z * z).sqrt() * rise).max(10.0)
    })
  }

  /// A bowl steepening outwards, from level at its centre to about 60
  /// degrees at its rim.
  fn bowl() -> HeightMap {
    map_from(160, 8.0, |x, z| 20.0 + (x * x + z * z) / 700.0)
  }

  #[test]
  fn no_tree_grows_steeper_than_its_species_allows() {
    let map = bowl();
    let surface = forest_surface(&map);
    let options = flora_options();
    let trees = build_tree_instances(&map, &surface, &options, 1.0);
    let fields = Fields::new(&map, &surface, &options, &Placement::default());

    assert!(trees.len() > 50);
    let mut steep = 0;

    for tree in &trees {
      let (x, z) = sample_of(&map, tree);
      let site = fields.site(x, z).expect("a tree stands on an allowed site");
      let species = TreeSpecies::ALL[tree.species_index() as usize];
      assert!(
        site.slope_degrees < species_niche(species).max_slope_degrees,
        "{species:?} on {} degrees",
        site.slope_degrees
      );
      steep += usize::from(site.slope_degrees > 38.0 && species != TreeSpecies::Shrub);
    }

    // Conifers stand on 38 to 45 degree slopes, as mountain forests do.
    assert!(steep > 0, "no tree above 38 degrees");
  }

  #[test]
  fn slope_thins_only_the_last_quarter_of_a_species_range() {
    let maxima: Vec<f32> = SPECIES_NICHES.iter().map(|n| n.max_slope_degrees).collect();
    assert_eq!(maxima, [38.0, 45.0, 45.0, 25.0, 40.0, 15.0, 30.0, 50.0]);

    for max in maxima {
      assert_eq!(slope_response(0.75 * max, max), 1.0);
      assert!((slope_response(0.875 * max, max) - 0.5).abs() < 1e-5);
      assert_eq!(slope_response(max, max), 0.0);
    }
  }

  #[test]
  fn shaded_slopes_hold_more_trees() {
    // A gentle cone of dry, mild meadow (moisture about 0.3, where trees
    // are short of water): the shaded, poleward (north, -z) half keeps
    // more moisture.
    let map = cone(22.0);
    let options = BiomeOptions {
      moisture_bias: -0.7,
      volcanism: 0.0,
      // One climate over the whole cone, so only its aspect differs.
      climate_scale_metres: 1.0e6,
      ..BiomeOptions::default()
    };
    let surface = classify(&map, &[], &options);
    let mut flora = flora_options();
    flora.max_instances = 1_000_000;
    // Trees north and south of the summit over four seeds: one seed's
    // groves, a few 180 m stands on each flank, can favour either side by
    // as much as the aspect does.
    let halves = |placement: &Placement| {
      (42..46).fold((0, 0), |(north, south), seed| {
        let flora = FloraOptions {
          seed_offset: seed,
          ..flora.clone()
        };
        let trees = build_tree_instances_with(&map, &surface, &flora, 1.0, placement);
        (
          north + trees.iter().filter(|tree| tree.position[2] < -40.0).count(),
          south + trees.iter().filter(|tree| tree.position[2] > 40.0).count(),
        )
      })
    };
    let (north, south) = halves(&Placement::default());

    assert!(
      north as f32 >= 1.2 * south as f32,
      "north {north}, south {south}"
    );

    // With the sun in the northern sky, the south side is the shaded one.
    let (north, south) = halves(&Placement {
      poleward: poleward_of_sun(-90.0),
      ..Placement::default()
    });
    assert!(
      south as f32 >= 1.2 * north as f32,
      "north {north}, south {south}"
    );
  }

  /// A plain with a river along z = 0: its riparian field falls off over
  /// 150 m, and trees keep clear of its 6 m channel.
  fn riverside(
    options: &BiomeOptions,
  ) -> (
    HeightMap,
    Vec<SurfaceSample>,
    crate::render::water::RiverNetwork,
  ) {
    // The plain falls gently (under 5 degrees) to the river.
    let map = map_from(160, 4.0, |_, z| 20.0 + z.abs() * 0.08);
    let half = 159.0 * 4.0 * 0.5;
    let riparian: Vec<u8> = (0..160 * 160)
      .map(|index| {
        let z = (index / 160) as f32 * 4.0 - half;
        let near = (1.0 - z.abs() / 150.0).max(0.0);
        (near * near * 255.0).round() as u8
      })
      .collect();
    let surface = classify(&map, &riparian, options);
    let rivers = crate::render::water::RiverNetwork {
      channels: vec![vec![[-400.0, 0.0, 3.0], [400.0, 0.0, 3.0]]],
      riparian,
      ..Default::default()
    };
    (map, surface, rivers)
  }

  fn near_and_far(trees: &[TreeInstance]) -> (f32, f32) {
    // Trees per metre of band either side of the river.
    let band = |low: f32, high: f32| {
      trees
        .iter()
        .filter(|tree| (low..high).contains(&tree.position[2].abs()))
        .count() as f32
        / (2.0 * (high - low))
    };
    (band(0.0, 30.0), band(185.0, 215.0))
  }

  #[test]
  fn ground_by_water_is_more_wooded() {
    let mut flora = flora_options();
    flora.max_instances = 1_000_000;
    // Cool temperate country, drier away from the river.
    let temperate = BiomeOptions {
      temperature_bias: -0.4,
      moisture_bias: -0.3,
      volcanism: 0.0,
      // One climate along the whole plain, so only the river differs.
      climate_scale_metres: 1.0e6,
      ..BiomeOptions::default()
    };
    let (map, surface, rivers) = riverside(&temperate);
    let placement = Placement {
      rivers: Some(&rivers),
      ..Placement::default()
    };
    let (near, far) = near_and_far(&build_tree_instances_with(
      &map, &surface, &flora, 1.0, &placement,
    ));
    assert!(near >= 1.15 * far, "near {near}, far {far}");

    // Savannah grows gallery woods along rivers.
    let dry = BiomeOptions {
      temperature_bias: 0.4,
      moisture_bias: -0.45,
      volcanism: 0.0,
      climate_scale_metres: 1.0e6,
      ..BiomeOptions::default()
    };
    let (map, surface, rivers) = riverside(&dry);
    let far_sample = &surface[5 * 160 + 80];
    assert_eq!(far_sample.biome_kind(), BiomeKind::SavannahExpanse);
    let placement = Placement {
      rivers: Some(&rivers),
      ..Placement::default()
    };
    let (near, far) = near_and_far(&build_tree_instances_with(
      &map, &surface, &flora, 1.0, &placement,
    ));
    assert!(near >= 1.15 * far, "near {near}, far {far}");
  }

  #[test]
  fn valley_floors_are_denser_than_ridges_with_rivers_off() {
    // Parallel ridges and valleys 240 m apart, falling gently to the
    // south, in dry country: water gathering in the valleys is what lets
    // trees grow there.
    let map = map_from(200, 6.0, |x, z| {
      60.0 + 12.0 * (x / 240.0 * std::f32::consts::TAU).cos() - z * 0.05
    });
    let options = BiomeOptions {
      moisture_bias: -0.7,
      volcanism: 0.0,
      climate_scale_metres: 1.0e6,
      ..BiomeOptions::default()
    };
    let surface = classify(&map, &[], &options);
    let mut flora = flora_options();
    flora.max_instances = 1_000_000;
    let (width, height) = (map.metadata.width, map.metadata.height);
    let drainage = crate::terrain::drainage::DrainageArea::d8(width, height, &map.heights);
    let placement = Placement {
      drainage: Some(&drainage),
      ..Placement::default()
    };
    // Trees within 30 m of a valley line (x = 120 + 240 k) or a ridge line
    // (x = 240 k).
    let counts = |trees: &[TreeInstance]| {
      let near = |offset: f32| {
        trees
          .iter()
          .filter(|t| ((t.position[0] - offset).rem_euclid(240.0) - 120.0).abs() > 90.0)
          .count()
      };
      (near(120.0), near(0.0))
    };
    let (valley, ridge) = counts(&build_tree_instances_with(
      &map, &surface, &flora, 1.0, &placement,
    ));
    assert!(
      valley as f32 > 1.2 * ridge as f32,
      "valley {valley}, ridge {ridge}"
    );

    // Without the drainage term the valleys have no such advantage.
    let (plain_valley, _) = counts(&build_tree_instances(&map, &surface, &flora, 1.0));
    assert!(
      valley as f32 > 1.15 * plain_valley as f32,
      "{valley} {plain_valley}"
    );
  }

  #[test]
  fn moisture_by_water_is_counted_once() {
    // Classification already holds 0.45 x riparian; placement adds only
    // the species' own water term, and nothing where there is no slope
    // or drainage.
    for (affinity, riparian) in [(1.0, 0.8), (-0.6, 0.8), (0.2, 0.0)] {
      let moisture = 0.5 + 0.45 * riparian;
      assert_eq!(
        effective_moisture(moisture, 1.0, 0.0, affinity, riparian),
        moisture + 0.35 * affinity * riparian
      );
    }

    // Water gathering from upstream adds up to 0.25.
    assert!((effective_moisture(0.5, 1.0e9, 0.0, 0.0, 0.0) - 0.75).abs() < 1e-6);
  }

  #[test]
  fn trees_grow_in_groves_not_an_even_scatter() {
    let map = map_from(200, 4.0, |_, _| 40.0);
    let surface = forest_surface(&map);
    let mut options = flora_options();
    options.density = 0.3;
    options.max_instances = 1_000_000;
    let trees = build_tree_instances(&map, &surface, &options, 1.0);
    let points: Vec<[f32; 2]> = trees
      .iter()
      .map(|t| [t.position[0], t.position[2]])
      .collect();
    // Clark and Evans: the mean nearest-neighbour distance over what a
    // random scatter of the same density would give, away from the edge.
    let inner = 300.0;
    let area = (2.0f32 * 398.0).powi(2);
    let density = points.len() as f32 / area;
    let mut total = 0.0;
    let mut count = 0;

    for (i, p) in points.iter().enumerate() {
      if p[0].abs() > inner || p[1].abs() > inner {
        continue;
      }

      let nearest = points
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != i)
        .map(|(_, q)| ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt())
        .fold(f32::MAX, f32::min);
      total += nearest;
      count += 1;
    }

    let ratio = (total / count as f32) / (0.5 / density.sqrt());
    assert!(
      count > 200 && ratio < 0.9,
      "Clark-Evans {ratio} over {count} trees"
    );
  }

  #[test]
  fn thinning_to_the_cap_leaves_no_stripes() {
    let map = map_from(120, 4.0, |_, _| 40.0);
    let surface = forest_surface(&map);
    let mut options = flora_options();
    options.max_instances = 1_000_000;
    let all = build_tree_instances(&map, &surface, &options, 1.0);
    options.max_instances = (all.len() / 2) as u32;
    let kept = build_tree_instances(&map, &surface, &options, 1.0);
    assert_eq!(kept.len(), all.len() / 2);

    let per_row = |trees: &[TreeInstance]| {
      // Rows of candidates, four at a time.
      let mut rows = vec![0.0f32; 30];

      for tree in trees {
        rows[(sample_of(&map, tree).1.round() as usize / 4).min(29)] += 1.0;
      }

      rows
    };
    let (all_rows, kept_rows) = (per_row(&all), per_row(&kept));
    // Each row keeps about half its trees: the spread of the kept share
    // is what binomial noise gives, and no run of rows is emptied.
    let mut excess = 0.0;
    let mut rows = 0.0;

    for (full, kept) in all_rows
      .iter()
      .zip(&kept_rows)
      .filter(|(full, _)| **full >= 20.0)
    {
      let expected = full * 0.5;
      excess += (kept - expected).powi(2) / (full * 0.25);
      rows += 1.0;
      assert!(*kept >= full * 0.2, "a row kept {kept} of {full}");
    }

    // Chi-squared over the rows: about 1 per row for binomial noise.
    assert!(
      rows > 20.0 && excess / rows < 2.0,
      "{} per row",
      excess / rows
    );
  }

  #[test]
  fn species_rules_replace_the_built_in_mix() {
    let map = flat_map(48, 50.0);
    let surface = forest_surface(&map);
    let biome = surface[48 * 24 + 24].biome_kind();
    let mut options = flora_options();
    options.species_rules = vec![FloraRule {
      biome,
      species: vec![vista_types::SpeciesWeight {
        species: vista_types::TreeSpeciesKind::Palm,
        weight: 2.0,
      }],
      density: 1.0,
    }];
    let trees = build_tree_instances(&map, &surface, &options, 1.0);

    assert!(!trees.is_empty());
    assert!(trees
      .iter()
      .all(|tree| tree.species_index() == TreeSpecies::Palm as u32));

    options.species_rules[0].species.clear();
    assert!(build_tree_instances(&map, &surface, &options, 1.0).is_empty());
  }

  #[test]
  fn disabled_flora_produces_no_instances() {
    let map = flat_map(32, 50.0);
    let mut options = flora_options();
    options.enabled = false;

    assert!(build_tree_instances(&map, &forest_surface(&map), &options, 1.0).is_empty());
  }

  #[test]
  fn underwater_terrain_produces_no_instances() {
    let map = flat_map(32, -10.0);
    let options = flora_options();

    assert!(build_tree_instances(&map, &forest_surface(&map), &options, 1.0).is_empty());
  }

  #[test]
  fn forest_above_sea_level_produces_instances() {
    let map = flat_map(32, 50.0);
    let options = flora_options();

    assert!(!build_tree_instances(&map, &forest_surface(&map), &options, 1.0).is_empty());
  }

  #[test]
  fn max_instances_is_respected() {
    let map = flat_map(64, 50.0);
    let mut options = flora_options();
    options.max_instances = 5;

    assert!(build_tree_instances(&map, &forest_surface(&map), &options, 1.0).len() <= 5);
  }

  #[test]
  fn hand_placed_trees_are_mature_first_variants_standing_upright() {
    let packed = [1.0, 2.0, 3.0, 1.0, 0.0, 0.5, 5.0, 0.0, 1.0];
    let tree = unpack_tree_placements(&packed).unwrap()[0];

    // Bits 10 to 19 stay clear: variant 0, mature, no lean.
    assert_eq!(tree.species & (0x3ff << 10), 0);
    assert_eq!(
      (tree.variant(), tree.age(), tree.lean()),
      (0, crate::render::tree_growth::Age::Mature as u32, (0, 0))
    );
  }

  #[test]
  fn root_flares_meet_grounded_ground_on_steep_slopes() {
    let size = 64u32;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 2.0,
      ..TerrainMetadata::default()
    };
    let centre = (31.0, 31.0);

    for degrees in [10.0f32, 15.0, 30.0, 40.0] {
      let mut map = HeightMap::flat(size, size, 0.0, metadata.clone());
      let rise = degrees.to_radians().tan();

      for (index, height) in map.heights.iter_mut().enumerate() {
        *height = (index as u32 % size) as f32 * 2.0 * rise;
      }

      for species in TreeSpecies::ALL {
        // Only where the species grows.
        if species == TreeSpecies::Shrub || degrees > species_niche(species).max_slope_degrees {
          continue;
        }

        // Grounded as the grounding pass's CPU mirror stands it.
        let (x, z) = (1.1, -0.7);
        let root = species_root_radius(species);
        let base = grounded_base(&map, centre, x, z, root);
        let mesh = crate::render::tree_models::build_species_mesh(species);

        // Every point of the trunk's lowest ring is under the ground, so
        // no gap shows round the trunk on the downhill side.
        for vertex in &mesh.vertices {
          if vertex.params[0] >= crate::render::tree_models::layers::FIRST_FOLIAGE
            || vertex.position[1] > -0.35
          {
            continue;
          }

          let ground = crate::render::terrain_mesh::mesh_surface_height(
            &map,
            centre,
            x + vertex.position[0],
            z + vertex.position[2],
          );
          assert!(
            base + vertex.position[1] < ground,
            "{species:?} at {degrees} degrees: the flare's foot {} is above the ground {ground}",
            base + vertex.position[1]
          );
        }
      }
    }
  }

  #[test]
  fn hand_placed_trees_unpack_with_their_ground_flag() {
    let packed = [
      1.0, 2.0, 3.0, 1.5, 0.25, 0.5, 3.0, 0.1, 0.0, //
      4.0, 5.0, 6.0, 2.0, 1.0, 0.2, 7.0, 0.9, 1.0,
    ];
    let trees = unpack_tree_placements(&packed).unwrap();

    assert_eq!(trees.len(), 2);
    assert_eq!(trees[0].position, [1.0, 2.0, 3.0]);
    assert_eq!(
      (
        trees[0].scale,
        trees[0].rotation,
        trees[0].tint,
        trees[0].dryness
      ),
      (1.5, 0.25, 0.5, 0.1)
    );
    assert_eq!(trees[0].species_index(), 3);
    assert!(!trees[0].grounded());
    assert_eq!(trees[1].species_index(), 7);
    assert!(trees[1].grounded());
    assert!(!trees[1].stunted());

    // Packing the instances again gives back the same numbers.
    let repacked: Vec<f32> = trees
      .iter()
      .flat_map(|tree| {
        [
          tree.position[0],
          tree.position[1],
          tree.position[2],
          tree.scale,
          tree.rotation,
          tree.tint,
          tree.species_index() as f32,
          tree.dryness,
          if tree.grounded() { 1.0 } else { 0.0 },
        ]
      })
      .collect();
    assert_eq!(repacked, packed);
  }

  #[test]
  fn malformed_tree_placements_are_rejected() {
    let tree = [0.0, 0.0, 0.0, 1.0, 0.0, 0.5, 0.0, 0.0, 0.0];
    assert!(unpack_tree_placements(&tree[..8])
      .unwrap_err()
      .contains("9 numbers per tree"));

    let mut bad = tree;
    bad[6] = 8.0;
    assert!(unpack_tree_placements(&bad)
      .unwrap_err()
      .contains("species"));
    bad[6] = 1.5;
    assert!(unpack_tree_placements(&bad).is_err());

    let mut bad = tree;
    bad[8] = 0.5;
    assert!(unpack_tree_placements(&bad)
      .unwrap_err()
      .contains("ground flag"));
    bad[8] = f32::NAN;
    assert!(unpack_tree_placements(&bad).is_err());
    assert!(unpack_tree_placements(&[]).unwrap().is_empty());
  }

  #[test]
  fn same_seed_is_deterministic() {
    let map = flat_map(32, 50.0);
    let options = flora_options();
    let surface = forest_surface(&map);

    let first = build_tree_instances(&map, &surface, &options, 1.0);
    let second = build_tree_instances(&map, &surface, &options, 1.0);

    assert_eq!(first, second);
  }

  fn cold_surface(map: &HeightMap, celsius: f32) -> Vec<SurfaceSample> {
    let options = BiomeOptions {
      mean_temperature_celsius: Some(celsius),
      volcanism: 0.0,
      ..BiomeOptions::default()
    };
    classify_surface(map, &generate_normals(map), None, &[], &options)
  }

  #[test]
  fn glaciers_are_treeless_and_tundra_grows_only_dwarf_shrubs() {
    let map = flat_map(96, 50.0);
    let options = flora_options();
    let glacier = cold_surface(&map, -20.0);

    assert!(glacier.iter().all(|sample| sample.is_glacier()));
    assert!(build_tree_instances(&map, &glacier, &options, 1.0).is_empty());

    let tundra = cold_surface(&map, 1.0);
    assert!(tundra.iter().all(|sample| sample.is_tundra()));
    let shrubs = build_tree_instances(&map, &tundra, &options, 1.0);

    // A tenth of full forest cover: one candidate per sample on this grid,
    // so well under a fifth of them become shrubs.
    assert!(!shrubs.is_empty());
    assert!(shrubs.len() * 5 < 96 * 96, "{} shrubs", shrubs.len());
    // The young shrub is the grown dwarf form, already knee high, so its
    // scale stays near 1.
    assert!(shrubs.iter().all(|tree| {
      tree.species_index() == TreeSpecies::Shrub as u32
        && (0.6..=1.2).contains(&tree.scale)
        && tree.age() == crate::render::tree_growth::Age::Young as u32
    }));
  }

  #[test]
  fn trees_sink_to_the_downhill_root_on_a_thirty_degree_plane() {
    let size = 64u32;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 4.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 0.0, metadata);
    let rise = 30f32.to_radians().tan();

    // Rising along +x at 30 degrees.
    for (index, height) in map.heights.iter_mut().enumerate() {
      *height = (index as u32 % size) as f32 * 4.0 * rise;
    }

    let centre = (31.0, 31.0);
    let root = 1.3;
    let (x, z) = (3.3, -2.1);
    let downhill = crate::render::terrain_mesh::mesh_surface_height(&map, centre, x - root, z);
    let base = grounded_base(&map, centre, x, z, root);

    assert!(
      (base - (downhill - 0.05 * root)).abs() < 1e-3,
      "{base} {downhill}"
    );
    // The trunk centre stands higher than the base by the slope over the
    // root radius: the uphill roots are buried, the downhill ones meet
    // the ground.
    let trunk = crate::render::terrain_mesh::mesh_surface_height(&map, centre, x, z);
    assert!((trunk - base - (root * rise + 0.05 * root)).abs() < 1e-3);
  }

  #[test]
  fn species_follow_the_biome() {
    assert_eq!(
      choose_species(BiomeKind::SwampWetlands, 0.6, 0.1),
      Some(TreeSpecies::Cypress)
    );
    assert_eq!(
      choose_species(BiomeKind::InnerJungle, 0.8, 0.1),
      Some(TreeSpecies::Jungle)
    );
    assert_eq!(
      choose_species(BiomeKind::CoastalBeach, 0.8, 0.5),
      Some(TreeSpecies::Palm)
    );
    assert_eq!(
      choose_species(BiomeKind::InnerForest, 0.2, 0.1),
      Some(TreeSpecies::Spruce)
    );
    assert_eq!(
      choose_species(BiomeKind::SavannahExpanse, 0.8, 0.1),
      Some(TreeSpecies::Acacia)
    );
    assert_eq!(choose_species(BiomeKind::Ocean, 0.5, 0.1), None);
  }
}
