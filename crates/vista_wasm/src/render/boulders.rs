//! Boulders and talus below rock outcrops: their meshes, built at start-up,
//! and where they lie, the CPU twin of `boulder_generate.wgsl`.
//!
//! Every potential boulder is a point on its own world-space lattice, 2 m
//! apart and hashed with its own salt (see `render/lattice.rs`), streamed
//! in 32 m tiles within the boulder distance. A point holds a boulder
//! where its rank is below the probability the soil model's talus field
//! gives (`soil::talus_byte`): 0.35 of the scree there, more at an
//! outcrop's foot. Sizes follow a power law from 0.3 to 3 m, larger the
//! further down the talus cone, as fallen blocks roll furthest. Nothing
//! lies in water, within a drawn channel's half width plus its clearance,
//! off the map, or where the talus field is 0 (sand, glacier, snow, river
//! beds). Each boulder is sunk by 25 to 40 % of its height into the drawn
//! ground and leans with the slope.
//!
//! Streams carry stones of their own, on a second lattice 1 m apart with
//! its own salt, which the same tiles hold: in the water wherever it runs
//! fast, sized by the force of the flow on its bed, a line of cobbles
//! along the wet margin, and a few washed-up blocks on the bank, never
//! more than 1.5 m from the water (see [`stone_at`]). The water shader
//! reads the same lattice, so foam breaks on the stones that are there.

use crate::maths::smoothstep;
use crate::render::lattice::{jittered, point_hash, unit};
use crate::render::vegetation::{in_channel, tile_first, GroundData};

/// Lattice pitch in metres.
pub const BOULDER_PITCH: f32 = 2.0;
/// Edge of a streamed boulder tile, in metres.
pub const BOULDER_TILE_METRES: f32 = 32.0;
/// Salt that separates the boulder lattice from the trees' and grass's.
pub const BOULDER_SALT: u32 = 0x7f4a_7c15;
/// Smallest and largest boulders, in metres across.
pub const BOULDER_SIZES: (f32, f32) = (0.3, 3.0);
/// The size distribution's power-law exponent: the share of boulders
/// larger than `s` falls as `s^-1.2`.
pub const SIZE_EXPONENT: f32 = 1.2;
/// Boulders keep this far outside lakes and rivers a sample wide, in
/// metres; drawn channels keep their own clearance (at least 1 m).
pub const BOULDER_WATER_CLEARANCE: f32 = 1.0;
/// Boulders lie at least this high above sea level, in metres.
pub const BOULDER_WATER_LINE: f32 = 0.3;
/// Most boulders drawn in a frame.
pub const MAX_BOULDERS: u32 = 20_000;
/// Boulders within this distance cast shadows, in metres.
pub const BOULDER_SHADOW_METRES: f32 = 150.0;
/// Mesh variants.
pub const VARIANTS: usize = 6;
/// A boulder's footprint radius per metre of its size: meshes are 1
/// across at most, so its half, and a little for the lean.
pub const FOOTPRINT: f32 = 0.55;

/// Stream stone lattice pitch in metres.
pub const STONE_PITCH: f32 = 1.0;
/// Salt that separates the stream stones' lattice from the boulders'.
pub const STONE_SALT: u32 = 0x1b87_3593;
/// Stones lie at most this far outside a stream's water, in metres.
pub const STONE_REACH: f32 = 1.5;
/// The cobbles along a fast stream's margin lie within this far of its
/// water's edge, in metres.
pub const COBBLE_METRES: f32 = 0.5;

/// A stream's bed stones from its speed (m/s), depth (m), slope and width
/// (m): their median size in metres and the share of the stone lattice's
/// points in its water that hold one. The force of the flow on the bed,
/// `tau = rho g d S`, moves grains up to `tau / (0.06 (rho_s - rho) g)`
/// across (Shields, 1936): the median here, at most a third of the width
/// (a brook's boulders are its steps, not walls across it), from 0.05 to
/// 3 m. Stones rise from none at 30 W/m^2 of stream power (`rho g v d S`)
/// to 40 % of the points at 300 W/m^2, and there are none where the water
/// runs under 0.5 m/s.
pub fn stone_bed(speed: f32, depth: f32, slope: f32, width: f32) -> [f32; 2] {
  let median = (depth * slope / (0.06 * 1.65))
    .min(width / 3.0)
    .clamp(0.05, 3.0);
  let power = 9810.0 * speed * depth * slope;
  let chance = 0.4 * smoothstep((power - 30.0) / 270.0) * smoothstep((speed - 0.5) / 0.3);
  [median, chance]
}

/// A stream stone's size from a uniform roll `u` and its stream's median
/// size: a power law, the share larger than `s` falling as `s^-2`, from
/// 0.71 to 3 times the median (so the median is about the stream's), and
/// one in fourteen over twice it, within 0.05 to 3 m.
pub fn stone_size(u: f32, median: f32) -> f32 {
  let tail = (0.71f32 / 3.0).powi(2);
  (0.71 * median / (1.0 - u * (1.0 - tail)).sqrt()).clamp(0.05, 3.0)
}

/// The stream stone at stone lattice point `(ix, iz)`, if one lies there:
/// its position, size and rank relative to its chance. In the water a
/// point holds one with its stream's chance; within [`COBBLE_METRES`] of
/// the water's edge, a cobble a third of the size with up to one and a
/// half times that chance (half at most); out to [`STONE_REACH`], a block
/// washed up with a third of it.
pub fn stone_candidate(bins: &[u32], seed: u32, ix: i32, iz: i32) -> Option<([f32; 2], f32, f32)> {
  let hash = point_hash(ix, iz, seed);
  let [x, z] = jittered(ix, iz, hash, STONE_PITCH);
  let [edge, median, chance] = crate::render::vegetation::channel_stones(bins, x, z, STONE_REACH)?;
  let (chance, scale) = if edge <= 0.0 {
    (chance, 1.0)
  } else if edge <= COBBLE_METRES {
    ((1.5 * chance).min(0.5), 0.33)
  } else {
    (chance / 3.0, 1.0)
  };
  let rank = unit(hash[2]);

  if rank >= chance {
    return None;
  }

  let traits = point_hash(ix, iz, seed ^ crate::render::lattice::TREE_TRAITS_SALT);
  Some((
    [x, z],
    (stone_size(unit(traits[0]), median) * scale).max(0.05),
    rank / chance,
  ))
}

/// The stream stone at stone lattice point `(ix, iz)`, standing on the
/// ground here and sunk by 30 to 60 % of its height into the bed.
pub fn stone_at(
  ground: &GroundData,
  bins: &[u32],
  rules: &BoulderRules,
  ix: i32,
  iz: i32,
) -> Option<Boulder> {
  let ([x, z], size, relative) = stone_candidate(bins, rules.stones, ix, iz)?;
  let traits = point_hash(
    ix,
    iz,
    rules.stones ^ crate::render::lattice::TREE_TRAITS_SALT,
  );
  let variant = traits[2] % VARIANTS as u32;
  let sink = (0.3 + 0.3 * unit(traits[1])) * size * rules.heights[variant as usize];
  Some(Boulder {
    position: [x, ground.height_at(x, z) - sink, z],
    size,
    sink,
    lean: [0.0; 2],
    code: variant as f32 + relative.min(0.999),
  })
}

/// Expected stream stones per heightmap texel of `ground`, from the
/// drawn `channels` and their `stones` (see `RiverNetwork`): each
/// segment's area out to [`STONE_REACH`] either side, at its stones'
/// chance.
pub fn stone_mass(
  ground: &GroundData,
  channels: &[Vec<[f32; 3]>],
  stones: &[Vec<[f32; 2]>],
) -> Vec<f32> {
  let mut mass = vec![0.0; (ground.width * ground.height) as usize];

  for (run, stones) in channels
    .iter()
    .zip(stones)
    .filter(|(run, stones)| run.len() == stones.len())
  {
    for (pair, chance) in run.windows(2).zip(stones.windows(2)) {
      let length = crate::maths::length2(pair[1][0] - pair[0][0], pair[1][1] - pair[0][1]);
      let area = length * (pair[0][2] + pair[1][2] + 2.0 * STONE_REACH);
      let texel = ground.nearest(
        0.5 * (pair[0][0] + pair[1][0]),
        0.5 * (pair[0][1] + pair[1][1]),
      );
      mass[texel] += area * 0.5 * (chance[0][1] + chance[1][1]) / (STONE_PITCH * STONE_PITCH);
    }
  }

  mass
}

/// One boulder (32 bytes), as the generator writes it and the boulder
/// pass draws it: `boulders.wgsl` reads these eight floats.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Boulder {
  /// Where its mesh's base stands, sunk into the ground.
  pub position: [f32; 3],
  /// Metres across.
  pub size: f32,
  /// How far its base lies below the drawn ground, in metres.
  pub sink: f32,
  /// The ground's slope along x and z (rise per metre), which it leans
  /// with.
  pub lean: [f32; 2],
  /// Mesh variant plus its rank relative to `p` (0 to 0.999), which the
  /// cull pass fades it out by.
  pub code: f32,
}

/// Everything the lattice needs beyond the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoulderRules {
  /// Lattice seed with [`BOULDER_SALT`].
  pub seed: u32,
  /// The stream stones' lattice seed, with [`STONE_SALT`].
  pub stones: u32,
  /// Each variant's height per metre across ([`BoulderMeshes::heights`]).
  pub heights: [f32; VARIANTS],
}

impl BoulderRules {
  /// Rules for a terrain seed and the meshes' heights.
  pub fn new(terrain_seed: u64, heights: [f32; VARIANTS]) -> Self {
    let seed = crate::render::lattice::lattice_seed(terrain_seed);
    Self {
      seed: seed ^ BOULDER_SALT,
      stones: seed ^ STONE_SALT,
      heights,
    }
  }
}

/// A boulder's size from a uniform roll `u` and how far down its talus
/// cone it lies (0 to 1): a power law truncated to [`BOULDER_SIZES`],
/// scaled from 0.75 at the outcrop's foot to 1.35 at the cone's end.
pub fn boulder_size(u: f32, position: f32) -> f32 {
  let (small, large) = BOULDER_SIZES;
  let tail = (small / large).powf(SIZE_EXPONENT);
  let size = small * (1.0 - u * (1.0 - tail)).powf(-1.0 / SIZE_EXPONENT);
  (size * (0.75 + 0.6 * position)).clamp(small, large)
}

/// The boulder candidate at lattice point `(ix, iz)`: its position, size
/// and rank relative to its probability, if one lies there. Water,
/// channels and the talus field are checked; not its height.
pub fn boulder_candidate(
  ground: &GroundData,
  bins: &[u32],
  seed: u32,
  ix: i32,
  iz: i32,
) -> Option<([f32; 2], f32, f32)> {
  let hash = point_hash(ix, iz, seed);
  let [x, z] = jittered(ix, iz, hash, BOULDER_PITCH);

  if !ground.on_map(x, z) {
    return None;
  }

  let talus = ground.banks[ground.nearest(x, z)][3];
  let (p, position) = crate::terrain::soil::talus_parts(talus);
  let rank = unit(hash[2]);

  if rank >= p
    || ground.height_at(x, z) <= ground.sea + BOULDER_WATER_LINE
    || ground.in_water(x, z, BOULDER_WATER_CLEARANCE)
    || in_channel(bins, x, z, true)
  {
    return None;
  }

  let traits = point_hash(ix, iz, seed ^ crate::render::lattice::TREE_TRAITS_SALT);
  Some(([x, z], boulder_size(unit(traits[0]), position), rank / p))
}

/// The boulder at lattice point `(ix, iz)`, standing on the ground here
/// (the generator stands it on the drawn mesh).
pub fn boulder_at(
  ground: &GroundData,
  bins: &[u32],
  rules: &BoulderRules,
  ix: i32,
  iz: i32,
) -> Option<Boulder> {
  let ([x, z], size, relative) = boulder_candidate(ground, bins, rules.seed, ix, iz)?;
  let traits = point_hash(
    ix,
    iz,
    rules.seed ^ crate::render::lattice::TREE_TRAITS_SALT,
  );
  let variant = traits[2] % VARIANTS as u32;
  let sink = (0.25 + 0.15 * unit(traits[1])) * size * rules.heights[variant as usize];
  let span = crate::render::vegetation::SLOPE_SPAN;
  let lean = [
    (ground.height_at(x + span, z) - ground.height_at(x - span, z)) / (2.0 * span),
    (ground.height_at(x, z + span) - ground.height_at(x, z - span)) / (2.0 * span),
  ];
  Some(Boulder {
    position: [x, ground.height_at(x, z) - sink, z],
    size,
    sink,
    lean,
    code: variant as f32 + relative.min(0.999),
  })
}

/// The boulders one streamed tile holds, as `boulder_generate.wgsl` fills
/// it: each 2 m boulder cell, then the four 1 m stream stone cells within
/// it.
pub fn tile_boulders(
  ground: &GroundData,
  bins: &[u32],
  rules: &BoulderRules,
  tile: [i32; 2],
) -> Vec<Boulder> {
  let (size, pitch) = (BOULDER_TILE_METRES as i32, BOULDER_PITCH as i32);
  let range = |t: i32| tile_first(t, size, pitch)..tile_first(t + 1, size, pitch);
  let mut boulders = Vec::new();

  for iz in range(tile[1]) {
    for ix in range(tile[0]) {
      boulders.extend(boulder_at(ground, bins, rules, ix, iz));

      for (sx, sz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        boulders.extend(stone_at(ground, bins, rules, 2 * ix + sx, 2 * iz + sz));
      }
    }
  }

  boulders
}

/// Whether world `(x, z)` lies within `margin` metres of a boulder's
/// footprint, so trees and grass leave room for it. `under_boulder` in
/// `generate_common.wgsl`.
pub fn under_boulder(
  ground: &GroundData,
  bins: &[u32],
  seed: u32,
  x: f32,
  z: f32,
  margin: f32,
) -> bool {
  let (cx, cz) = (
    (x / BOULDER_PITCH).floor() as i32,
    (z / BOULDER_PITCH).floor() as i32,
  );

  (-1..=1).any(|dz| {
    (-1..=1).any(|dx| {
      boulder_candidate(ground, bins, seed, cx + dx, cz + dz).is_some_and(|([bx, bz], size, _)| {
        crate::maths::length2(bx - x, bz - z) < size * FOOTPRINT + margin
      })
    })
  })
}

/// Expected boulders per 32 m tile, from the talus field, and stream
/// stones from `stones`, per texel ([`stone_mass`], or empty).
pub fn boulder_mass(ground: &GroundData, stones: &[f32]) -> crate::render::vegetation::TileMass {
  let area = ground.texel_metres * ground.texel_metres / (BOULDER_PITCH * BOULDER_PITCH);
  crate::render::vegetation::TileMass::build(ground, BOULDER_TILE_METRES, BOULDER_PITCH, &|texel| {
    let talus = ground.banks.get(texel).map_or(0, |banks| banks[3]);
    crate::terrain::soil::talus_parts(talus).0 * area + stones.get(texel).copied().unwrap_or(0.0)
  })
}

/// Streamed boulders fill tiles out to `distance`, each slot holding a
/// tile's boulders with room for half as many again as the densest.
pub fn boulder_stream(
  mass: crate::render::vegetation::TileMass,
  distance: f32,
) -> crate::render::vegetation::Stream {
  use crate::render::lattice::{count_bound, TileLayout, TileRing};
  let capacity = count_bound(1.5 * mass.most());
  let layout = TileLayout::new(BOULDER_TILE_METRES, &[distance], &|_| capacity);
  crate::render::vegetation::Stream {
    ring: TileRing::new(&layout),
    layout,
    mass,
    floor: 0.0,
    radius: f32::MAX,
    hands_over: false,
  }
}

/// One boulder mesh vertex (24 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BoulderVertex {
  /// Position: the base at y 0, at most 1 across.
  pub position: [f32; 3],
  /// Unit normal.
  pub normal: [f32; 3],
}

/// Levels of detail per variant: 20, 80 and 320 triangles, sharing the
/// vertices (a coarser level uses the first 12 or 42 of them).
pub const LODS: usize = 3;

/// The six boulder variants in one vertex and index buffer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoulderMeshes {
  /// Every variant's vertices, one after another.
  pub vertices: Vec<BoulderVertex>,
  /// Indices, variant by variant, each finest level first.
  pub indices: Vec<u16>,
  /// Per variant and level: first index, index count and base vertex.
  pub ranges: [[(u32, u32, i32); LODS]; VARIANTS],
  /// Each variant's height per metre across.
  pub heights: [f32; VARIANTS],
}

/// A solid noise from about -1 to 1 over the unit sphere: two value
/// noises on crossed planes, which the binary already has.
fn noise3(seed: u64, p: [f32; 3]) -> f32 {
  let value_noise = crate::maths::value_noise;
  0.6 * value_noise(seed, p[0] + 0.6 * p[2], p[1])
    + 0.6 * value_noise(seed ^ 0x51ed, p[2] - 0.4 * p[0], p[1] * 1.3 + p[0])
}

impl BoulderMeshes {
  /// Six fractured blocks: an icosphere of 162 vertices, displaced by a
  /// three-octave noise with sharpened peaks, squashed, and cut flat on
  /// one or two sides.
  pub fn build() -> Self {
    use crate::maths::{add, cross, dot, length2, mul, normalise, sub};
    let (base, faces) = icosphere();
    let mut meshes = Self::default();

    for variant in 0..VARIANTS {
      let seed = 0x9b05_688c_2b3e_6c1f ^ (variant as u64).wrapping_mul(0x1f83_d9ab_fb41_bd6b);
      let roll = |salt: u64| (crate::maths::hash_u64(seed ^ salt) >> 40) as f32 / 16_777_216.0;
      let squash = 0.5 + 0.3 * roll(1);
      // One or two cuts, each a plane at 0.55 to 0.75 from the centre,
      // facing sideways or a little up or down.
      let cuts = 1 + usize::from(roll(2) > 0.5);
      let mut points = Vec::with_capacity(base.len());

      for v in &base {
        let mut radius = 1.0;
        let mut amplitude = 0.22;
        let mut scale = 1.6;

        for octave in 0..3 {
          // Sharpened peaks: ridges where the noise crosses zero.
          let n = noise3(seed ^ octave, v.map(|c| c * scale));
          radius += amplitude * (1.0 - 2.0 * n.abs());
          amplitude *= 0.45;
          scale *= 2.0;
        }

        let mut p = [v[0] * radius, v[1] * radius * squash, v[2] * radius];

        for cut in 0..cuts as u64 {
          let (a, b) = (
            roll(10 + cut) * std::f32::consts::TAU,
            0.5 * roll(20 + cut) - 0.3,
          );
          let normal = [a.cos() * b.cos(), b.sin(), a.sin() * b.cos()];
          let reach = dot(p, normal) - (0.55 + 0.2 * roll(30 + cut));

          if reach > 0.0 {
            p = sub(p, mul(normal, reach));
          }
        }

        points.push(p);
      }

      // The base at y 0, at most 1 across.
      let (mut low, mut across) = (f32::MAX, 0.0f32);

      for p in &points {
        low = low.min(p[1]);
        across = across.max(2.0 * length2(p[0], p[2]));
      }

      let mut normals = vec![[0.0f32; 3]; points.len()];

      for p in &mut points {
        *p = [p[0] / across, (p[1] - low) / across, p[2] / across];
        meshes.heights[variant] = meshes.heights[variant].max(p[1]);
      }

      for face in &faces[2] {
        let [a, b, c] = face.map(|i| points[i as usize]);
        let n = cross(sub(b, a), sub(c, a));

        for i in face {
          normals[*i as usize] = add(normals[*i as usize], n);
        }
      }

      let base_vertex = meshes.vertices.len() as i32;

      for (position, normal) in points.iter().zip(&normals) {
        meshes.vertices.push(BoulderVertex {
          position: *position,
          normal: normalise(*normal),
        });
      }

      for (lod, level) in faces.iter().rev().enumerate() {
        let first = meshes.indices.len() as u32;

        for face in level {
          meshes.indices.extend(face.map(|i| i as u16));
        }

        meshes.ranges[variant][lod] = (first, level.len() as u32 * 3, base_vertex);
      }
    }

    meshes
  }
}

/// A unit icosphere subdivided twice (162 vertices), and its faces at each
/// level: 20, 80 and 320. Midpoints are appended, so each level's
/// vertices start with the previous level's.
fn icosphere() -> (Vec<[f32; 3]>, [Vec<[u32; 3]>; 3]) {
  let t = (1.0 + 5f32.sqrt()) / 2.0;
  let mut vertices: Vec<[f32; 3]> = [
    [-1.0, t, 0.0],
    [1.0, t, 0.0],
    [-1.0, -t, 0.0],
    [1.0, -t, 0.0],
    [0.0, -1.0, t],
    [0.0, 1.0, t],
    [0.0, -1.0, -t],
    [0.0, 1.0, -t],
    [t, 0.0, -1.0],
    [t, 0.0, 1.0],
    [-t, 0.0, -1.0],
    [-t, 0.0, 1.0],
  ]
  .map(crate::maths::normalise)
  .to_vec();
  let base: Vec<[u32; 3]> = vec![
    [0, 11, 5],
    [0, 5, 1],
    [0, 1, 7],
    [0, 7, 10],
    [0, 10, 11],
    [1, 5, 9],
    [5, 11, 4],
    [11, 10, 2],
    [10, 7, 6],
    [7, 1, 8],
    [3, 9, 4],
    [3, 4, 2],
    [3, 2, 6],
    [3, 6, 8],
    [3, 8, 9],
    [4, 9, 5],
    [2, 4, 11],
    [6, 2, 10],
    [8, 6, 7],
    [9, 8, 1],
  ];
  let mut subdivide = |faces: &[[u32; 3]]| {
    // At most 480 edges, once at start-up: a list is small and fast
    // enough.
    let mut midpoints: Vec<((u32, u32), u32)> = Vec::new();
    let mut middle = |a: u32, b: u32, vertices: &mut Vec<[f32; 3]>| {
      let edge = (a.min(b), a.max(b));

      if let Some((_, index)) = midpoints.iter().find(|(key, _)| *key == edge) {
        return *index;
      }

      let (p, q) = (vertices[a as usize], vertices[b as usize]);
      vertices.push(crate::maths::normalise(crate::maths::add(p, q)));
      midpoints.push((edge, vertices.len() as u32 - 1));
      vertices.len() as u32 - 1
    };
    faces
      .iter()
      .flat_map(|[a, b, c]| {
        let (ab, bc, ca) = (
          middle(*a, *b, &mut vertices),
          middle(*b, *c, &mut vertices),
          middle(*c, *a, &mut vertices),
        );
        [[*a, ab, ca], [*b, bc, ab], [*c, ca, bc], [ab, bc, ca]]
      })
      .collect::<Vec<_>>()
  };
  let middle = subdivide(&base);
  let fine = subdivide(&middle);
  (vertices, [base, middle, fine])
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::terrain::soil::talus_byte;
  use vista_types::TerrainMetadata;

  /// A level map at `height` metres whose talus field says `talus`
  /// everywhere, with no water.
  fn ground(height: f32, talus: u8) -> (crate::terrain::HeightMap, GroundData) {
    let metadata = TerrainMetadata {
      width: 64,
      height: 64,
      metres_per_sample: 4.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let map = crate::terrain::HeightMap::flat(64, 64, height, metadata);
    let mut ground = GroundData::of(&map);
    ground.banks = vec![[255, 0, 0, talus]; (ground.width * ground.height) as usize];
    (map, ground)
  }

  fn rules() -> BoulderRules {
    BoulderRules::new(7, BoulderMeshes::build().heights)
  }

  /// Every boulder over the map's tiles.
  fn all(ground: &GroundData, bins: &[u32], rules: &BoulderRules) -> Vec<Boulder> {
    (-4..4)
      .flat_map(|tz| (-4..4).map(move |tx| [tx, tz]))
      .flat_map(|tile| tile_boulders(ground, bins, rules, tile))
      .collect()
  }

  #[test]
  fn the_meshes_are_fractured_blocks_with_shared_levels_of_detail() {
    let meshes = BoulderMeshes::build();
    assert_eq!(meshes.vertices.len(), VARIANTS * 162);

    for variant in 0..VARIANTS {
      let counts = meshes.ranges[variant].map(|(_, count, _)| count / 3);
      assert_eq!(counts, [320, 80, 20]);
      // Squashed: lower than they are across, and never flatter than a
      // third.
      assert!(
        (0.3..0.95).contains(&meshes.heights[variant]),
        "{:?}",
        meshes.heights
      );
      let (first, count, base) = meshes.ranges[variant][2];
      let coarse = &meshes.indices[first as usize..(first + count) as usize];
      assert!(coarse.iter().all(|i| *i < 12) && base >= 0);
      let vertices = &meshes.vertices[base as usize..base as usize + 162];
      assert!(vertices.iter().all(|v| v.position[1] >= -1e-5
        && crate::maths::length2(v.position[0], v.position[2]) <= 0.5 + 1e-4));
      // Not a sphere: the radius from the centre varies by a fifth or more.
      let centre = [0.0, meshes.heights[variant] * 0.5, 0.0];
      let radii: Vec<f32> = vertices
        .iter()
        .map(|v| {
          crate::maths::length2(
            crate::maths::length2(v.position[0], v.position[2]),
            v.position[1] - centre[1],
          )
        })
        .collect();
      let (low, high) = radii
        .iter()
        .fold((f32::MAX, 0.0f32), |(a, b), r| (a.min(*r), b.max(*r)));
      assert!(high > low * 1.2, "{low} {high}");
    }

    assert_ne!(meshes.heights[0], meshes.heights[1]);
  }

  #[test]
  fn sizes_follow_a_power_law_with_many_small_and_few_large() {
    let mut sizes: Vec<f32> = (0..10_000)
      .map(|i| boulder_size((i as f32 + 0.5) / 10_000.0, 0.5))
      .collect();
    sizes.sort_by(f32::total_cmp);
    assert!(sizes[5_000] < 0.8, "median {}", sizes[5_000]);
    assert!(sizes[0] >= BOULDER_SIZES.0 && sizes[9_999] <= BOULDER_SIZES.1);
    assert!(sizes[9_900] > 2.0, "{}", sizes[9_900]);
    // Blocks further down the cone are larger.
    assert!(boulder_size(0.9, 1.0) > boulder_size(0.9, 0.0));
  }

  #[test]
  fn boulders_are_sunk_by_a_quarter_to_two_fifths_of_their_height() {
    let (_, ground) = ground(20.0, talus_byte(0.5, 0.5));
    let bins = crate::render::vegetation::channel_bins(&ground_map(), None);
    let rules = rules();
    let boulders = all(&ground, &bins, &rules);
    assert!(boulders.len() > 100);

    for boulder in &boulders {
      let height = boulder.size * rules.heights[boulder.code as usize];
      let sunk = ground.height_at(boulder.position[0], boulder.position[2]) - boulder.position[1];
      assert!((sunk - boulder.sink).abs() < 1e-4);
      assert!(
        sunk >= 0.25 * height - 1e-4 && sunk <= 0.4 * height + 1e-4,
        "{sunk} of {height}"
      );
    }
  }

  fn ground_map() -> crate::terrain::HeightMap {
    ground(20.0, 0).0
  }

  #[test]
  fn no_boulders_without_talus_under_water_or_off_the_map() {
    let bins = crate::render::vegetation::channel_bins(&ground_map(), None);
    assert!(all(&ground(20.0, 0).1, &bins, &rules()).is_empty());
    assert!(all(&ground(-3.0, talus_byte(0.5, 0.5)).1, &bins, &rules()).is_empty());

    // Lakes: the distance-to-water field reads 0.
    let (_, mut lake) = ground(20.0, talus_byte(0.5, 0.5));
    lake.banks.iter_mut().for_each(|texel| texel[0] = 0);
    assert!(all(&lake, &bins, &rules()).is_empty());

    let (_, land) = ground(20.0, talus_byte(0.5, 0.5));
    assert!(all(&land, &bins, &rules())
      .iter()
      .all(|b| land.on_map(b.position[0], b.position[2])));
  }

  #[test]
  fn the_talus_field_excludes_sand_glacier_snow_and_river_beds() {
    use crate::terrain::biomes::{SurfaceSample, MAT_GRAVEL, MAT_ICE, MAT_SAND};
    let mut sample = SurfaceSample {
      talus: talus_byte(0.5, 0.2),
      ..SurfaceSample::default()
    };
    sample.materials[4] = 200;
    assert_ne!(sample.talus_here(), 0);

    for (material, weight) in [(MAT_SAND, 120), (MAT_ICE, 200), (MAT_GRAVEL, 10)] {
      let mut covered = sample;
      covered.materials[material] = weight;
      assert_eq!(covered.talus_here(), 0, "{material}");
    }

    let glacier = SurfaceSample {
      biome: vista_types::BiomeKind::IceArctic as u8,
      permanent_snow: 255,
      ..sample
    };
    assert_eq!(glacier.talus_here(), 0);
    assert_eq!(
      SurfaceSample {
        river: 255,
        ..sample
      }
      .talus_here(),
      0
    );
  }

  #[test]
  fn boulders_keep_out_of_every_drawn_channel_even_the_narrowest() {
    // A brook 0.8 m across, far narrower than a 4 m sample, winding across
    // talus.
    let (map, ground) = ground(20.0, talus_byte(0.6, 0.5));
    let brook: Vec<[f32; 3]> = (0..240)
      .map(|i| {
        let x = -120.0 + i as f32;
        [x, 6.0 * (x / 25.0 * std::f32::consts::TAU).sin(), 0.4]
      })
      .collect();
    let rivers = crate::render::water::RiverNetwork {
      channels: vec![brook.clone()],
      ..Default::default()
    };
    let bins = crate::render::vegetation::channel_bins(&map, Some(&rivers));
    let rules = rules();
    let boulders = all(&ground, &bins, &rules);
    // How far from the brook's centreline, less its half width.
    let clear = |x: f32, z: f32| {
      brook
        .windows(2)
        .map(|pair| {
          -crate::render::flora::Water {
            a: [pair[0][0], pair[0][1]],
            b: [pair[1][0], pair[1][1]],
            half: [pair[0][2], pair[1][2]],
            clearance: 0.0,
            stones: [0.0; 2],
            band: 0.0,
          }
          .intrusion(x, z)
        })
        .fold(f32::MAX, f32::min)
    };

    assert!(!boulders.is_empty());
    // At least the half width plus 1 m from the water.
    assert!(boulders
      .iter()
      .all(|b| clear(b.position[0], b.position[2]) >= BOULDER_WATER_CLEARANCE));
    // The boulders beside it are still there.
    assert!(boulders
      .iter()
      .any(|b| clear(b.position[0], b.position[2]) < 4.0));
    // The GPU reads the same bins the CPU does, with the channels'
    // clearance.
    let wgsl = include_str!("../shaders/generate_common.wgsl");
    assert!(wgsl.contains("in_water(m, xz, BOULDER_WATER_CLEARANCE) || in_channel(xz, 1.0)"));
  }

  #[test]
  fn trees_and_grass_leave_room_for_boulders() {
    let (_, talus) = ground(20.0, talus_byte(0.6, 0.5));
    let bins = crate::render::vegetation::channel_bins(&ground_map(), None);
    let rules = rules();

    for boulder in all(&talus, &bins, &rules) {
      let [x, _, z] = boulder.position;
      assert!(under_boulder(&talus, &bins, rules.seed, x, z, 0.0));
      assert!(under_boulder(
        &talus,
        &bins,
        rules.seed,
        x + boulder.size * FOOTPRINT * 0.9,
        z,
        0.0
      ));
    }

    let (_, bare) = ground(20.0, 0);
    assert!(!under_boulder(&bare, &bins, rules.seed, 3.0, 5.0, 1.0));
  }

  /// A straight stream 3 m wide across a flat, talus-free map, 0.5 m
  /// deep, with its bed stones from `speed` and `slope`, and the bins the
  /// generators read.
  fn stream(speed: f32, slope: f32) -> (GroundData, Vec<u32>, Vec<[f32; 3]>) {
    let (map, ground) = ground(20.0, 0);
    let run: Vec<[f32; 3]> = (0..=60)
      .map(|i| [-60.0 + 2.0 * i as f32, 3.0, 1.5])
      .collect();
    let rivers = crate::render::water::RiverNetwork {
      channels: vec![run.clone()],
      stones: vec![vec![stone_bed(speed, 0.5, slope, 3.0); run.len()]],
      ..Default::default()
    };
    let bins = crate::render::vegetation::channel_bins(&map, Some(&rivers));
    (ground, bins, run)
  }

  /// The stream stones over the map's tiles.
  fn stones_of(ground: &GroundData, bins: &[u32], rules: &BoulderRules) -> Vec<Boulder> {
    (-4..4)
      .flat_map(|tz| (-4..4).map(move |tx| [tx, tz]))
      .flat_map(|[tx, tz]| {
        let range = |t: i32| tile_first(t, 32, 1)..tile_first(t + 1, 32, 1);
        range(tz)
          .flat_map(move |iz| range(tx).map(move |ix| (ix, iz)))
          .filter_map(|(ix, iz)| stone_at(ground, bins, rules, ix, iz))
          .collect::<Vec<_>>()
      })
      .collect()
  }

  #[test]
  fn stream_stones_grow_with_stream_power_and_stay_by_the_water() {
    let rules = rules();
    let mut medians = Vec::new();

    for slope in [0.01, 0.03, 0.08, 0.2] {
      let (ground, bins, run) = stream(2.0, slope);
      let stones = stones_of(&ground, &bins, &rules);
      assert!(!stones.is_empty(), "no stones at {slope}");
      let mut sizes: Vec<f32> = stones.iter().map(|s| s.size).collect();
      medians.push(crate::terrain::river_metrics::median(&mut sizes).unwrap());

      for stone in &stones {
        let edge = run
          .windows(2)
          .map(|pair| {
            -crate::render::flora::Water {
              a: [pair[0][0], pair[0][1]],
              b: [pair[1][0], pair[1][1]],
              half: [pair[0][2], pair[1][2]],
              clearance: 0.0,
              stones: [0.0; 2],
              band: 0.0,
            }
            .intrusion(stone.position[0], stone.position[2])
          })
          .fold(f32::MAX, f32::min);
        assert!(
          edge <= STONE_REACH + 1e-4,
          "a stone {edge} m from the water"
        );
        // Sunk by 30 to 60 % of its height.
        let height = stone.size * rules.heights[stone.code as usize];
        assert!(stone.sink >= 0.3 * height - 1e-4 && stone.sink <= 0.6 * height + 1e-4);
      }
    }

    assert!(
      medians.windows(2).all(|pair| pair[1] > pair[0]),
      "{medians:?}"
    );

    // Slow water moves no stones: none at 0.4 m/s, whatever the slope.
    let (ground, bins, _) = stream(0.4, 0.2);
    assert!(stones_of(&ground, &bins, &rules).is_empty());
    assert_eq!(stone_bed(0.4, 1.0, 0.3, 3.0)[1], 0.0);
    // The Shields median: 10 d S, within 0.05 to 3 m, and a third of the
    // width at most.
    assert!((stone_bed(2.0, 0.5, 0.1, 3.0)[0] - 0.505).abs() < 0.01);
    assert_eq!(stone_bed(2.0, 3.0, 0.5, 20.0)[0], 3.0);
    assert!((stone_bed(2.0, 0.5, 0.2, 1.2)[0] - 0.4).abs() < 1e-5);
    // About half the stones are larger than the median, one in fourteen
    // over twice it.
    let mut sizes: Vec<f32> = (0..1000)
      .map(|i| stone_size((i as f32 + 0.5) / 1000.0, 0.4))
      .collect();
    sizes.sort_by(f32::total_cmp);
    assert!((sizes[500] - 0.4).abs() < 0.02, "{}", sizes[500]);
    let large = sizes.iter().filter(|s| **s > 0.8).count();
    assert!((60..90).contains(&large), "{large}");
  }

  /// The stones `stream_stone` in `water.wgsl` weighs at `p`, line for
  /// line: every stone of the lattice within `reach` cells.
  fn shader_stones(p: [f32; 2], bed: [f32; 2], seed: u32, reach: i32) -> Vec<([f32; 2], f32)> {
    let base = [p[0].floor() as i32, p[1].floor() as i32];
    let mut stones = Vec::new();

    for dy in -reach..=reach {
      for dx in -reach..=reach {
        let (ix, iz) = (base[0] + dx, base[1] + dy);
        let hash = point_hash(ix, iz, seed);

        if unit(hash[2]) < bed[1] {
          let traits = point_hash(ix, iz, seed ^ crate::render::lattice::TREE_TRAITS_SALT);
          stones.push((
            jittered(ix, iz, hash, 1.0),
            0.5 * stone_size(unit(traits[0]), bed[0]),
          ));
        }
      }
    }

    stones
  }

  #[test]
  fn the_water_breaks_on_the_stones_the_meshes_draw() {
    let rules = rules();
    let (ground, bins, _) = stream(2.0, 0.08);
    let bed = stone_bed(2.0, 0.5, 0.08, 3.0);
    // In the water, away from the stream's ends.
    let wet = |x: f32, z: f32| (z - 3.0).abs() < 1.0 && x.abs() < 55.0;
    let stones: Vec<Boulder> = stones_of(&ground, &bins, &rules)
      .into_iter()
      .filter(|s| wet(s.position[0], s.position[2]))
      .collect();
    assert!(stones.len() > 10);

    for stone in &stones {
      let at = [stone.position[0], stone.position[2]];
      let weighed = shader_stones(at, bed, rules.stones, 1);
      assert!(weighed.iter().any(|(centre, radius)| {
        crate::maths::length2(centre[0] - at[0], centre[1] - at[1]) < 1e-5
          && (2.0 * radius - stone.size).abs() < 1e-5
      }));

      // And every stone the shader weighs in the water is a mesh.
      for (centre, radius) in weighed.iter().filter(|(c, _)| wet(c[0], c[1])) {
        assert!(stones.iter().any(|s| {
          crate::maths::length2(s.position[0] - centre[0], s.position[2] - centre[1]) < 1e-5
            && (s.size - 2.0 * radius).abs() < 1e-5
        }));
      }
    }

    let wgsl = include_str!("../shaders/water.wgsl");

    for line in [
      "clamp(min(depth * slope / (0.06 * 1.65), width / 3.0), 0.05, 3.0)",
      "0.4 * smoothstep(30.0, 300.0, power) * smoothstep(0.5, 0.8, speed)",
      "0.71 * median / sqrt(1.0 - u * (1.0 - 0.056))",
      "seed ^ 0x2545f491u",
    ] {
      assert!(wgsl.contains(line), "the water shader no longer has {line}");
    }
  }

  #[test]
  fn the_generator_uses_the_same_lattice_constants() {
    let wgsl = format!(
      "{}{}",
      include_str!("../shaders/generate_common.wgsl"),
      include_str!("../shaders/boulder_generate.wgsl")
    );

    for line in [
      format!("const BOULDER_PITCH: f32 = {BOULDER_PITCH:.1};"),
      "tile_first(job.tile.x, 32, 2)".to_string(),
      format!("const BOULDER_WATER_LINE: f32 = {BOULDER_WATER_LINE};"),
      format!("const BOULDER_WATER_CLEARANCE: f32 = {BOULDER_WATER_CLEARANCE:.1};"),
      format!("const SIZE_EXPONENT: f32 = {SIZE_EXPONENT};"),
      format!("const FOOTPRINT: f32 = {FOOTPRINT};"),
      format!("const MAX_BOULDERS: u32 = {MAX_BOULDERS}u;"),
      format!(
        "const TALUS_PROBABILITY: f32 = {};",
        crate::terrain::soil::MAX_TALUS_PROBABILITY
      ),
      "0.25 + 0.15 * unit(traits.y)".to_string(),
      "0.3 + 0.3 * unit(traits.y)".to_string(),
      format!("const STONE_REACH: f32 = {STONE_REACH:.1};"),
      format!("const COBBLE_METRES: f32 = {COBBLE_METRES};"),
      "chance = min(1.5 * chance, 0.5);".to_string(),
      "scale = 0.33;".to_string(),
      "2 * cell + vec2<i32>(k % 2, k / 2)".to_string(),
      "traits.z % VARIANTS".to_string(),
      "0.75 + 0.6 * position".to_string(),
    ] {
      assert!(
        wgsl.contains(&line),
        "the boulder generator no longer has {line}"
      );
    }

    assert_eq!(BOULDER_SALT, 0x7f4a_7c15);
    assert_eq!(STONE_PITCH, 1.0);
    assert_eq!(crate::render::vegetation::BIN_WORDS, 11);
    assert!(wgsl.contains("const BIN_WORDS: u32 = 11u;"));
    assert_eq!(VARIANTS, 6);
  }
}
