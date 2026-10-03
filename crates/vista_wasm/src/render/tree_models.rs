//! Tree species models.
//!
//! Every species is grown when the renderer starts (see
//! `render/tree_growth.rs`): four variants, each in four age classes
//! (young, mature, old and wind-shaped krummholz), and each at two levels
//! of detail, merged into one [`TreeLibrary`]. Fixed seeds mean every run
//! produces byte-identical meshes, so shipping the generator costs a few
//! kilobytes of code instead of megabytes of model data, and there is no
//! per-frame cost: the meshes are uploaded once and drawn instanced.
//!
//! Per-tree variety (size, rotation, colour, lean, variant and age class)
//! comes from instance data. A host's custom model ([`mesh_from_arrays`])
//! replaces every variant of its species.

use std::f32::consts::TAU;

use crate::render::flora::{species_root_radius, ROOT_RADII};
use crate::render::tree_growth::{grow_meshes, Age, AGES, LODS, VARIANTS};

/// Number of modelled species.
pub const SPECIES_COUNT: usize = 8;

/// Tree species, in the order used by instance data and shaders.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[repr(u32)]
pub enum TreeSpecies {
  /// Broad, spreading deciduous oak.
  Oak = 0,
  /// Tall pine with a high, irregular crown.
  Pine = 1,
  /// Dense, conical spruce.
  Spruce = 2,
  /// Coconut palm with an arching frond crown.
  Palm = 3,
  /// Tall buttressed rainforest emergent.
  Jungle = 4,
  /// Bald cypress with a flared base and knees.
  Cypress = 5,
  /// Flat-topped savannah acacia.
  Acacia = 6,
  /// Low leafy shrub.
  Shrub = 7,
}

impl TreeSpecies {
  /// Every species in index order.
  pub const ALL: [TreeSpecies; SPECIES_COUNT] = [
    Self::Oak,
    Self::Pine,
    Self::Spruce,
    Self::Palm,
    Self::Jungle,
    Self::Cypress,
    Self::Acacia,
    Self::Shrub,
  ];
}

/// Flora texture array layers produced by `texture_gen.wgsl`.
pub mod layers {
  /// Deeply fissured oak bark.
  pub const BARK_OAK: f32 = 0.0;
  /// Plated, reddish pine bark.
  pub const BARK_PINE: f32 = 1.0;
  /// Ringed palm trunk.
  pub const BARK_PALM: f32 = 2.0;
  /// Smooth, grey tropical bark.
  pub const BARK_SMOOTH: f32 = 3.0;
  /// A cluster of ovate, serrated broadleaves.
  pub const LEAF_BROAD: f32 = 4.0;
  /// Large, glossy ovate tropical leaves.
  pub const LEAF_TROPICAL: f32 = 5.0;
  /// Pine needle bundles.
  pub const NEEDLES: f32 = 6.0;
  /// A single palm frond with pinnate leaflets.
  pub const PALM_FROND: f32 = 7.0;
  /// Tiny bipinnate leaflets (acacia).
  pub const LEAF_FINE: f32 = 8.0;
  /// Hanging moss strands.
  pub const MOSS: f32 = 9.0;
  /// A pair of fern fronds and a young, coiled one, for the forest floor.
  pub const FERN: f32 = 10.0;
  /// A low clump of small leaves, for the forest floor.
  pub const UNDERGROWTH: f32 = 11.0;
  /// A cluster of lobed oak leaves.
  pub const LEAF_OAK: f32 = 12.0;
  /// Flat spruce needle sprays.
  pub const SPRUCE: f32 = 13.0;
  /// Scale-leaf sprays (cypress).
  pub const CYPRESS: f32 = 14.0;
  /// Number of flora texture layers a model may use.
  pub const COUNT: u32 = 15;
  /// First foliage layer; layers at or above this are alpha-tested.
  pub const FIRST_FOLIAGE: f32 = 4.0;
  /// The leaf normal maps follow the colour layers, one per leaf layer:
  /// see [`normal_layer`].
  pub const NORMAL_LAYERS: u32 = 8;
  /// Every layer of the flora texture array.
  pub const TOTAL: u32 = COUNT + NORMAL_LAYERS;

  /// The layer holding a leaf layer's normal map (in its red and green),
  /// or `None` for layers without one. Mirrored by `leaf_normal_layer` in
  /// `trees.wgsl` and `texture_gen.wgsl`.
  pub fn normal_layer(layer: u32) -> Option<u32> {
    let slot = match layer {
      4..=8 => layer - 4,
      12..=14 => layer - 7,
      _ => return None,
    };
    Some(COUNT + slot)
  }
}

/// One tree mesh vertex (56 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TreeVertex {
  /// Model-space position in metres, base of the trunk at the origin.
  pub position: [f32; 3],
  /// Unit normal (xyz) as signed normalised 16-bit integers. Foliage
  /// normals bend away from the crown centre, so crowns shade as volumes.
  /// w holds the branch level (0 trunk, 1 limb, 2 twig, 3 leaf) / 4 plus
  /// 0.2 times a clump's colour variation (0 to 1), or, on bark, 0.2 for
  /// a cypress knee that dry ground sinks.
  pub normal: [i16; 4],
  /// Texture coordinates. On leaves, v runs from the twig (0) outwards.
  pub uv: [f32; 2],
  /// x: flora texture layer, y: stiffness (0 free .. 1 rigid), z: baked
  /// ambient occlusion, w: the wind phase of the vertex's branch.
  pub params: [f32; 4],
  /// Where the vertex's limb leaves the trunk, which it bobs around (the
  /// origin on the trunk).
  pub pivot: [f32; 3],
}

/// Pack a unit normal and a value from 0 to 1 into [`TreeVertex::normal`].
pub fn pack_normal(normal: [f32; 3], w: f32) -> [i16; 4] {
  let snorm = |value: f32| (value.clamp(-1.0, 1.0) * 32767.0).round() as i16;
  [
    snorm(normal[0]),
    snorm(normal[1]),
    snorm(normal[2]),
    snorm(w),
  ]
}

/// Unpack [`TreeVertex::normal`].
pub fn unpack_normal(packed: [i16; 4]) -> ([f32; 3], f32) {
  let unit = |value: i16| f32::from(value) / 32767.0;
  (
    [unit(packed[0]), unit(packed[1]), unit(packed[2])],
    unit(packed[3]),
  )
}

/// A generated species mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeMesh {
  /// Vertices.
  pub vertices: Vec<TreeVertex>,
  /// Triangle list indices.
  pub indices: Vec<u32>,
  /// Height of the tallest vertex in metres.
  pub height: f32,
  /// Largest horizontal distance of any vertex from the trunk axis.
  pub radius: f32,
}

impl TreeMesh {
  /// Radius of the trunk at breast height (0.8 to 1.5 m up): half the
  /// width its bark spans there, which a leaning trunk keeps. Meshes with
  /// no bark at that height take 2 % of their height.
  pub fn trunk_radius(&self) -> f32 {
    let (mut low, mut high) = ([f32::MAX; 2], [f32::MIN; 2]);

    for v in &self.vertices {
      if v.params[0] < layers::FIRST_FOLIAGE && (0.8..=1.5).contains(&v.position[1]) {
        for (axis, value) in [v.position[0], v.position[2]].into_iter().enumerate() {
          low[axis] = low[axis].min(value);
          high[axis] = high[axis].max(value);
        }
      }
    }

    if low[0] > high[0] {
      return (self.height * 0.02).max(0.02);
    }

    ((high[0] - low[0] + high[1] - low[1]) * 0.25).max(0.02)
  }

  /// The flora texture layers this mesh samples, one bit per layer.
  pub fn flora_layers(&self) -> u32 {
    self.vertices.iter().fold(0, |mask, vertex| {
      let layer = vertex.params[0]
        .round()
        .clamp(0.0, (layers::COUNT - 1) as f32);
      mask | 1 << layer as u32
    })
  }
}

/// Mesh slots per species: variants x age classes x levels of detail.
pub const MESHES_PER_SPECIES: usize = VARIANTS * AGES * LODS;
/// Mesh slots in a library.
pub const MESH_SLOTS: usize = SPECIES_COUNT * MESHES_PER_SPECIES;

/// One view in the impostor atlas: nine per variant, in a 3 x 3 grid per
/// layer, with a colour layer and a normal layer per variant. Trees switch
/// to impostors at 150 m, where even the tallest stand under 160 pixels
/// high on a 1080p screen.
pub const IMPOSTOR_CELL: [u32; 2] = [80, 160];
/// Mip levels of the impostor atlas: each view's margin keeps its mips
/// from bleeding into the next down to the smallest.
pub const IMPOSTOR_MIPS: u32 = 5;

/// Bytes of the impostor atlas with its mips at `variants` a species.
pub fn impostor_bytes(variants: usize) -> u64 {
  let layers = (SPECIES_COUNT * variants * 2) as u64;

  (0..IMPOSTOR_MIPS)
    .map(|level| {
      let width = u64::from((IMPOSTOR_CELL[0] * 3) >> level).max(1);
      let height = u64::from((IMPOSTOR_CELL[1] * 3) >> level).max(1);
      width * height * 4 * layers
    })
    .sum()
}

/// The library slot of a species' variant, age class and level of
/// detail. `tree_cull.wgsl` numbers its mesh lists the same way.
pub fn mesh_slot(species: usize, variant: usize, age: usize, lod: usize) -> usize {
  ((species * VARIANTS + variant) * AGES + age) * LODS + lod
}

/// One species' meshes: grown, or the host's model.
#[derive(Clone, Debug, PartialEq)]
pub enum SpeciesModel {
  /// Grown variants, each with its age classes, each at both levels of
  /// detail, indexed `variant * AGES + age`.
  Grown {
    /// The species.
    species: TreeSpecies,
    /// Variants grown (1 to 4).
    variants: usize,
    /// The meshes.
    meshes: Vec<[TreeMesh; LODS]>,
  },
  /// A custom model, used for every variant, age and level of detail.
  Custom(TreeMesh),
}

impl SpeciesModel {
  /// Grow `variants` (1 to 4) variants of a species.
  pub fn grow(species: TreeSpecies, variants: usize) -> Self {
    let variants = variants.clamp(1, VARIANTS);
    let mut meshes = Vec::with_capacity(variants * AGES);

    for variant in 0..variants {
      for age in Age::ALL {
        // Shrubs are never stunted, so they grow no krummholz.
        if species == TreeSpecies::Shrub && age == Age::Krummholz {
          meshes.push([TreeMesh::default(), TreeMesh::default()]);
        } else {
          meshes.push(grow_meshes(species, variant, age));
        }
      }
    }

    Self::Grown {
      species,
      variants,
      meshes,
    }
  }

  /// The mesh of a variant, age class and level of detail. Variants past
  /// those grown wrap around.
  pub fn mesh(&self, variant: usize, age: usize, lod: usize) -> &TreeMesh {
    match self {
      Self::Grown {
        species,
        variants,
        meshes,
      } => {
        let age = if *species == TreeSpecies::Shrub && age == Age::Krummholz as usize {
          Age::Mature as usize
        } else {
          age.min(AGES - 1)
        };
        &meshes[(variant % variants) * AGES + age][lod.min(LODS - 1)]
      }
      Self::Custom(mesh) => mesh,
    }
  }

  /// Variants with their own meshes.
  pub fn variants(&self) -> usize {
    match self {
      Self::Grown { variants, .. } => *variants,
      Self::Custom(_) => 1,
    }
  }

  /// The mesh a variant's impostor is rendered from: its mature tree.
  pub fn impostor_mesh(&self, variant: usize) -> &TreeMesh {
    self.mesh(variant, Age::Mature as usize, 0)
  }

  /// The flora texture layers the meshes sample, one bit per layer.
  pub fn flora_layers(&self) -> u32 {
    match self {
      Self::Grown { meshes, .. } => meshes
        .iter()
        .fold(0, |mask, pair| mask | pair[0].flora_layers()),
      Self::Custom(mesh) => mesh.flora_layers(),
    }
  }
}

/// All species meshes merged into one vertex and index buffer.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeLibrary {
  /// Merged vertices.
  pub vertices: Vec<TreeVertex>,
  /// Merged indices (relative to each mesh's base vertex).
  pub indices: Vec<u32>,
  /// `(first_index, index_count, base_vertex)` per [`mesh_slot`].
  pub ranges: Vec<(u32, u32, i32)>,
  /// `(height, radius)` per species in metres: the largest of its
  /// meshes.
  pub bounds: [(f32, f32); SPECIES_COUNT],
  /// Root radius per species in metres. Trees are sunk to the lowest
  /// ground within it.
  pub roots: [f32; SPECIES_COUNT],
}

impl TreeLibrary {
  /// Triangles in one mesh slot.
  pub fn triangles(&self, slot: usize) -> u32 {
    self.ranges[slot].1 / 3
  }
}

/// Grow every species with `variants` variants and merge them.
pub fn build_tree_library(variants: usize) -> TreeLibrary {
  let models: Vec<SpeciesModel> = TreeSpecies::ALL
    .iter()
    .map(|species| SpeciesModel::grow(*species, variants))
    .collect();
  build_library(&models)
}

/// The mesh the procedural model of `species` starts from: its first
/// variant, mature, at full detail.
pub fn build_species_mesh(species: TreeSpecies) -> TreeMesh {
  let [full, _] = grow_meshes(species, 0, Age::Mature);
  full
}

/// Merge one model per species into a library. A mesh several slots
/// share is stored once.
pub fn build_library(models: &[SpeciesModel]) -> TreeLibrary {
  let mut vertices = Vec::new();
  let mut indices = Vec::new();
  let mut ranges = vec![(0, 0, 0); MESH_SLOTS];
  let mut bounds = [(1.0, 1.0); SPECIES_COUNT];
  let mut roots = [0.5; SPECIES_COUNT];
  let mut stored: Vec<(*const TreeMesh, (u32, u32, i32))> = Vec::new();

  for (species, model) in models.iter().take(SPECIES_COUNT).enumerate() {
    roots[species] = match model {
      SpeciesModel::Grown { species, .. } => species_root_radius(*species),
      SpeciesModel::Custom(mesh) => mesh.trunk_radius() * ROOT_RADII,
    };
    let (mut height, mut radius) = (0.5f32, 0.25f32);

    for variant in 0..VARIANTS {
      for age in 0..AGES {
        for lod in 0..LODS {
          let mesh = model.mesh(variant, age, lod);
          height = height.max(mesh.height);
          radius = radius.max(mesh.radius);
          let key = mesh as *const TreeMesh;
          let range = match stored.iter().find(|(pointer, _)| *pointer == key) {
            Some((_, range)) => *range,
            None => {
              let range = (
                indices.len() as u32,
                mesh.indices.len() as u32,
                vertices.len() as i32,
              );
              vertices.extend_from_slice(&mesh.vertices);
              indices.extend_from_slice(&mesh.indices);
              stored.push((key, range));
              range
            }
          };
          ranges[mesh_slot(species, variant, age, lod)] = range;
        }
      }
    }

    bounds[species] = (height, radius);
  }

  TreeLibrary {
    vertices,
    indices,
    ranges,
    bounds,
    roots,
  }
}

/// Largest custom tree mesh accepted, in vertices.
pub const MAX_CUSTOM_TREE_VERTICES: usize = 65_536;
/// Most indices a custom tree mesh may hold: 131,072 triangles, twice the
/// vertex limit, which a closed mesh of that many vertices needs.
pub const MAX_CUSTOM_TREE_INDICES: usize = 3 * 131_072;
/// Furthest a custom tree's vertex may lie from its base on any axis, in
/// metres. Several times the tallest real tree, it keeps the mesh bounds
/// and the wind maths finite.
pub const MAX_CUSTOM_TREE_METRES: f32 = 1_000.0;

/// Build a tree mesh from host-supplied arrays, validating everything.
///
/// `positions` and `normals` hold three floats per vertex, `uvs` two, and
/// `indices` three per triangle. `layers` optionally gives a flora texture
/// layer per vertex (default 0, opaque bark); layers at or above
/// [`layers::FIRST_FOLIAGE`] are alpha-tested. `wind` optionally gives a
/// sway weight from 0 (rigid) to 1 per vertex; by default it grows with
/// height. Units are metres with the base of the trunk at the origin.
///
/// The layered wind's data is derived: every vertex pivots at the origin,
/// bark below 30 % of the height is trunk and above it limb, foliage
/// flutters, and stiffness is 1 minus the sway weight.
pub fn mesh_from_arrays(
  positions: &[f32],
  normals: &[f32],
  uvs: &[f32],
  indices: &[u32],
  texture_layers: Option<&[f32]>,
  wind: Option<&[f32]>,
) -> Result<TreeMesh, String> {
  if positions.is_empty() || !positions.len().is_multiple_of(3) {
    return Err("positions must hold three numbers per vertex.".to_string());
  }

  let count = positions.len() / 3;

  if count > MAX_CUSTOM_TREE_VERTICES {
    return Err(format!(
      "a tree model may have at most {MAX_CUSTOM_TREE_VERTICES} vertices, but it has {count}."
    ));
  }

  if normals.len() != count * 3 {
    return Err(format!(
      "normals must hold three numbers per vertex ({}), but they hold {}.",
      count * 3,
      normals.len()
    ));
  }

  if uvs.len() != count * 2 {
    return Err(format!(
      "uvs must hold two numbers per vertex ({}), but they hold {}.",
      count * 2,
      uvs.len()
    ));
  }

  if indices.is_empty()
    || !indices.len().is_multiple_of(3)
    || indices.len() > MAX_CUSTOM_TREE_INDICES
  {
    return Err(format!(
      "indices must hold three indices per triangle, from 3 to {MAX_CUSTOM_TREE_INDICES} in all, but they hold {}.",
      indices.len()
    ));
  }

  if let Some(index) = indices.iter().find(|index| **index as usize >= count) {
    return Err(format!(
      "indices must refer to existing vertices, 0 to {}, but one is {index}.",
      count - 1
    ));
  }

  // `abs() <= limit` is false for NaN, so this also rejects NaN.
  if !positions
    .iter()
    .all(|value| value.abs() <= MAX_CUSTOM_TREE_METRES)
    || normals.iter().chain(uvs).any(|value| !value.is_finite())
  {
    return Err(format!(
      "positions must be finite and within {MAX_CUSTOM_TREE_METRES} m of the base, and normals and uvs finite."
    ));
  }

  if let Some(layers) = texture_layers {
    if layers.len() != count {
      return Err("textureLayers must hold one layer per vertex.".to_string());
    }

    if layers
      .iter()
      .any(|layer| !layer.is_finite() || *layer < 0.0 || *layer >= layers::COUNT as f32)
    {
      return Err(format!(
        "textureLayers must be between 0 and {}.",
        layers::COUNT - 1
      ));
    }
  }

  if let Some(weights) = wind {
    if weights.len() != count || weights.iter().any(|value| !(0.0..=1.0).contains(value)) {
      return Err("wind must hold one value from 0 to 1 per vertex.".to_string());
    }
  }

  let height = (0..count)
    .map(|i| positions[i * 3 + 1])
    .fold(0.0f32, f32::max)
    .max(0.1);
  let mut vertices = Vec::with_capacity(count);
  let (mut top, mut radius) = (0.0f32, 0.0f32);

  for i in 0..count {
    let position = [positions[i * 3], positions[i * 3 + 1], positions[i * 3 + 2]];
    let layer = texture_layers.map_or(0.0, |layers| layers[i].floor());
    let rise = (position[1] / height).clamp(0.0, 1.0);
    let sway = wind.map_or(rise * rise, |weights| weights[i]);
    let level = if layer >= layers::FIRST_FOLIAGE {
      3.0
    } else if rise < 0.3 {
      0.0
    } else {
      1.0
    };
    let normal = crate::render::tree_growth::normalise([
      normals[i * 3],
      normals[i * 3 + 1],
      normals[i * 3 + 2],
    ]);
    top = top.max(position[1]);
    radius = radius.max((position[0].powi(2) + position[2].powi(2)).sqrt());
    vertices.push(TreeVertex {
      position,
      normal: pack_normal(normal, level * 0.25),
      uv: [uvs[i * 2], uvs[i * 2 + 1]],
      params: [layer, 1.0 - sway, 1.0, (i as f32 * 0.618).fract() * TAU],
      pivot: [0.0; 3],
    });
  }

  Ok(TreeMesh {
    vertices,
    indices: indices.to_vec(),
    height: top,
    radius,
  })
}

/// The direction the prevailing wind blows towards (x, z): krummholz
/// streams this way, and trees lean with it. Trees sway this way while
/// the weather has no wind of its own.
pub const PREVAILING_WIND: [f32; 2] = [0.8, 0.6];
/// The layered wind never moves a vertex further than this share of the
/// tree's height.
pub const WIND_BOUND: f32 = 0.08;

/// One vertex's inputs to the layered wind, in metres around the tree's
/// base (after the tree's rotation and scale).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindSample {
  /// The vertex.
  pub position: [f32; 3],
  /// Its limb's pivot.
  pub pivot: [f32; 3],
  /// Its (bent) normal.
  pub normal: [f32; 3],
  /// The tree's height.
  pub height: f32,
  /// Branch level, 0 to 3.
  pub level: f32,
  /// Stiffness, 0 to 1.
  pub stiffness: f32,
  /// Its branch's phase.
  pub phase: f32,
  /// Along a leaf card, from its twig (0) to its tip (1).
  pub along: f32,
  /// Whether the vertex is foliage.
  pub leaf: bool,
  /// Seconds.
  pub time: f32,
  /// The tree's position (x, z).
  pub instance: [f32; 2],
  /// `FloraOptions::windStrength`, 0 to 1 (the weather sets it from its
  /// wind).
  pub strength: f32,
  /// The weather's wind (x, z) in metres per second.
  pub wind: [f32; 2],
  /// The species' trunk bend and leaf flutter (`tree_growth::species_wind`).
  pub species: (f32, f32),
}

/// The layered wind: the trunk bends from its base, limbs bob about
/// their pivots and leaves flutter about their twigs. A port of
/// `wind_offset` in `trees.wgsl`, line for line. Nothing moves at the
/// ground, and nothing further than [`WIND_BOUND`] of the height.
pub fn wind_offset(s: &WindSample) -> [f32; 3] {
  let w = s.strength.clamp(0.0, 1.0) * 1.6;

  if w <= 1e-4 {
    return [0.0; 3];
  }

  let speed = (s.wind[0] * s.wind[0] + s.wind[1] * s.wind[1]).sqrt();
  let dir = if speed > 0.1 {
    [s.wind[0] / speed, s.wind[1] / speed]
  } else {
    let l = (PREVAILING_WIND[0].powi(2) + PREVAILING_WIND[1].powi(2)).sqrt();
    [PREVAILING_WIND[0] / l, PREVAILING_WIND[1] / l]
  };
  let t = s.time;
  let h = s.height.max(0.1);
  let y = s.position[1].max(0.0);
  // Gusts travel downwind across the forest.
  let gust_phase = (s.instance[0] * dir[0] + s.instance[1] * dir[1]) * 0.012 - t * 0.9;
  let gust = 0.55 + 0.45 * gust_phase.sin() * (gust_phase * 0.37 + 1.3).sin();

  // 1. The trunk bends from its base at 0.2 to 0.5 Hz.
  let frequency = (1.6 / h.sqrt()).clamp(0.2, 0.5);
  let tree_phase = s.instance[0] * 0.13 + s.instance[1] * 0.11;
  let sway = 0.7 + 0.3 * (TAU * frequency * t + tree_phase).sin();
  let amplitude = s.species.0 * w * w * h * h * 0.0025 * gust * sway;
  let bend = (y / h) * (y / h) * amplitude;
  let mut d = [dir[0] * bend, -0.08 * bend, dir[1] * bend];
  // Branches and leaves come to rest at the ground.
  let rise = (y / (0.15 * h)).clamp(0.0, 1.0);

  // 2. Limbs bob about their pivots at 0.8 to 1.5 Hz.
  if s.level >= 0.5 {
    let rel = [
      s.position[0] - s.pivot[0],
      s.position[1] - s.pivot[1],
      s.position[2] - s.pivot[2],
    ];
    let reach = (rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2]).sqrt();
    let frequency = 0.8 + 0.7 * (s.phase * 0.159).fract();
    let angle = w
      * (1.0 - s.stiffness.clamp(0.0, 1.0))
      * 0.1
      * gust
      * (TAU * frequency * t + s.phase + tree_phase).sin();
    // Up and down, and a little along the wind.
    let bob = angle * reach * rise / 1.25f32.sqrt();
    d = [
      d[0] + dir[0] * 0.5 * bob,
      d[1] + bob,
      d[2] + dir[1] * 0.5 * bob,
    ];
  }

  // 3. Leaves flutter about their twigs at 3 to 6 Hz.
  if s.leaf {
    let frequency = 3.0 + 3.0 * (s.phase * 0.37 + 0.5).fract();
    let flutter = w
      * s.species.1
      * 0.12
      * (TAU * frequency * t + s.phase * 3.0 + s.position[1] * 1.7).sin()
      * s.along
      * rise;
    d = [
      d[0] + s.normal[0] * flutter,
      d[1] + s.normal[1] * flutter,
      d[2] + s.normal[2] * flutter,
    ];
  }

  let moved = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
  let bound = WIND_BOUND * h;

  if moved > bound {
    let k = bound / moved;
    d = [d[0] * k, d[1] * k, d[2] * k];
  }

  d
}

const _: () = assert!(std::mem::size_of::<TreeVertex>() == 56);

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_impostor_atlas_keeps_to_its_memory_budget() {
    // The atlas may take 64 MB at the most variants.
    assert!(impostor_bytes(VARIANTS) <= 64 * 1024 * 1024);
    // Each view's cell halves cleanly at every level, so no level mixes
    // two views.
    let smallest = 1 << (IMPOSTOR_MIPS - 1);
    assert_eq!(IMPOSTOR_CELL[0] % smallest, 0);
    assert_eq!(IMPOSTOR_CELL[1] % smallest, 0);
  }

  #[test]
  fn roots_come_from_the_species_table() {
    let library = build_tree_library(1);

    for species in TreeSpecies::ALL {
      let root = library.roots[species as usize];
      // From a shrub's stems to a buttressed rainforest emergent.
      assert!((0.05..=5.0).contains(&root), "{species:?}: {root}");
      assert_eq!(root, species_root_radius(species), "{species:?}");
    }

    // A broad rainforest trunk roots wider than a palm's slim stem.
    assert!(
      library.roots[TreeSpecies::Jungle as usize] > library.roots[TreeSpecies::Palm as usize]
    );
  }

  #[test]
  fn every_mesh_is_valid() {
    let library = build_tree_library(VARIANTS);

    for slot in 0..MESH_SLOTS {
      let (first, count, base) = library.ranges[slot];
      assert!(count > 0 && count % 3 == 0, "slot {slot}");
      let indices = &library.indices[first as usize..(first + count) as usize];
      let highest = indices.iter().copied().max().unwrap_or(0) as i32 + base;
      assert!((highest as usize) < library.vertices.len(), "slot {slot}");
    }

    assert!(library.vertices.iter().all(|vertex| vertex
      .position
      .iter()
      .chain(vertex.uv.iter())
      .chain(vertex.params.iter())
      .chain(vertex.pivot.iter())
      .all(|value| value.is_finite())));
  }

  #[test]
  fn the_library_keeps_to_its_budgets() {
    let models: Vec<SpeciesModel> = TreeSpecies::ALL
      .iter()
      .map(|species| SpeciesModel::grow(*species, VARIANTS))
      .collect();
    let mut lod0 = 0;

    for (species, model) in models.iter().enumerate() {
      let SpeciesModel::Grown { meshes, .. } = model else {
        unreachable!();
      };

      for (slot, [full, light]) in meshes.iter().enumerate() {
        lod0 += full.vertices.len();
        let (full, light) = (full.indices.len() / 3, light.indices.len() / 3);
        assert!(
          light as f32 <= full as f32 * 0.45,
          "species {species} mesh {slot}: {light} against {full} triangles"
        );
      }
    }

    assert!(lod0 <= 900_000, "{lod0} LOD0 vertices");
  }

  #[test]
  fn foliage_uses_foliage_layers() {
    let oak = build_species_mesh(TreeSpecies::Oak);

    assert!(oak
      .vertices
      .iter()
      .any(|vertex| vertex.params[0] >= layers::FIRST_FOLIAGE));
    assert!(oak
      .vertices
      .iter()
      .any(|vertex| vertex.params[0] < layers::FIRST_FOLIAGE));
  }

  #[test]
  fn leaf_layers_have_normal_maps() {
    let mut seen = Vec::new();

    for layer in [4, 5, 6, 7, 8, 12, 13, 14] {
      let normals = layers::normal_layer(layer).unwrap();
      assert!((layers::COUNT..layers::TOTAL).contains(&normals));
      assert!(!seen.contains(&normals));
      seen.push(normals);
    }

    assert_eq!(layers::normal_layer(4), Some(15));
    assert_eq!(layers::normal_layer(14), Some(22));
    assert_eq!(layers::normal_layer(0), None);
    assert_eq!(layers::normal_layer(9), None);
  }

  #[test]
  fn custom_meshes_are_validated() {
    let positions = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 5.0, 0.0];
    let normals = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
    let uvs = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
    let mesh = mesh_from_arrays(&positions, &normals, &uvs, &[0, 1, 2], None, None).unwrap();

    assert_eq!(mesh.vertices.len(), 3);
    assert!((mesh.height - 5.0).abs() < 1e-5);
    assert!(mesh_from_arrays(&positions, &normals, &uvs, &[0, 1, 3], None, None).is_err());
    assert!(mesh_from_arrays(&positions, &normals[..6], &uvs, &[0, 1, 2], None, None).is_err());
    assert!(mesh_from_arrays(
      &positions,
      &normals,
      &uvs,
      &[0, 1, 2],
      Some(&[0.0, 15.0, 0.0]),
      None
    )
    .is_err());
    assert!(mesh_from_arrays(&[f32::NAN; 9], &normals, &uvs, &[0, 1, 2], None, None).is_err());
  }

  #[test]
  fn custom_meshes_get_wind_data() {
    let positions = [0.0, 0.0, 0.0, 1.0, 0.5, 0.0, 0.0, 5.0, 0.0, 1.0, 4.0, 1.0];
    let normals = [0.0, 1.0, 0.0].repeat(4);
    let uvs = [0.0; 8];
    let mesh = mesh_from_arrays(
      &positions,
      &normals,
      &uvs,
      &[0, 1, 2, 1, 2, 3],
      Some(&[0.0, 0.0, 0.0, layers::LEAF_BROAD]),
      Some(&[0.0, 0.1, 0.8, 1.0]),
    )
    .unwrap();
    let levels: Vec<f32> = mesh
      .vertices
      .iter()
      .map(|vertex| (unpack_normal(vertex.normal).1 * 4.0 + 0.01).floor())
      .collect();

    // Trunk below 30 % of the height, limb above it, and foliage.
    assert_eq!(levels, [0.0, 0.0, 1.0, 3.0]);

    for vertex in &mesh.vertices {
      assert!(vertex.pivot.iter().all(|value| value.is_finite()));
      assert_eq!(vertex.pivot, [0.0; 3]);
      assert!((0.0..=1.0).contains(&vertex.params[1]));
      assert!(vertex.params[3].is_finite());
    }

    // Stiffness is 1 minus the sway weight.
    assert!((mesh.vertices[2].params[1] - 0.2).abs() < 1e-6);
  }

  #[test]
  fn the_wind_is_still_at_the_ground_and_bounded() {
    let storm = |position: [f32; 3], time: f32, level: f32, leaf: bool| WindSample {
      position,
      pivot: [0.0, 4.0, 0.0],
      normal: [0.0, 0.0, 1.0],
      height: 20.0,
      level,
      stiffness: 0.0,
      phase: 1.3,
      along: 1.0,
      leaf,
      time,
      instance: [120.0, -40.0],
      strength: 1.0,
      wind: [20.0, 5.0],
      species: (1.2, 1.0),
    };

    for step in 0..200 {
      let time = step as f32 * 0.137;

      for (level, leaf) in [(0.0, false), (1.0, false), (2.0, false), (3.0, true)] {
        assert_eq!(
          wind_offset(&storm([3.0, 0.0, 1.0], time, level, leaf)),
          [0.0; 3],
          "level {level}"
        );

        for height in [1.0, 5.0, 12.0, 20.0, 24.0] {
          let d = wind_offset(&storm([6.0, height, -2.0], time, level, leaf));
          let moved = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
          assert!(moved <= WIND_BOUND * 20.0 + 1e-4, "{moved} at {height} m");
        }
      }
    }

    // The layers move by different amounts: the canopy more than the
    // trunk, and leaves more than the limb they hang from.
    let at = |level, leaf| {
      let d = wind_offset(&WindSample {
        strength: 0.5,
        ..storm([2.0, 12.0, 0.0], 0.4, level, leaf)
      });
      (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    };
    assert_ne!(at(0.0, false), at(2.0, false));
    assert_ne!(at(2.0, false), at(3.0, true));
  }

  #[test]
  fn normals_pack_and_unpack() {
    let (normal, w) = unpack_normal(pack_normal([0.6, -0.8, 0.0], 0.95));
    assert!((normal[0] - 0.6).abs() < 1e-4 && (normal[1] + 0.8).abs() < 1e-4);
    assert!((w - 0.95).abs() < 1e-4);
  }
}
