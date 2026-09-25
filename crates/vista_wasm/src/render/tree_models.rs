//! Procedural tree species models.
//!
//! Each species is modelled once, from code, when the renderer starts:
//! trunks and branches are tapered generalised cylinders that follow
//! curved growth paths, and foliage is built from alpha-tested leaf,
//! needle, and frond cards textured with procedurally baked leaf textures
//! (see `shaders/texture_gen.wgsl`). A fixed per-species seed means every
//! run produces byte-identical meshes, so shipping the generator costs a
//! few kilobytes of code instead of megabytes of model data, and there is
//! no per-frame cost: the meshes are uploaded once and drawn instanced.
//!
//! Per-tree variety (size, rotation, colour, lean) comes from instance
//! data, not from separate meshes.

use std::f32::consts::{FRAC_PI_2, TAU};

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
  /// Bald cypress with a flared base and hanging moss.
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
  /// Smooth, pale tropical bark.
  pub const BARK_SMOOTH: f32 = 3.0;
  /// Cluster of broad, ovate leaves.
  pub const LEAF_BROAD: f32 = 4.0;
  /// Large, glossy tropical leaves.
  pub const LEAF_TROPICAL: f32 = 5.0;
  /// Conifer needle spray.
  pub const NEEDLES: f32 = 6.0;
  /// A single palm frond with leaflets.
  pub const PALM_FROND: f32 = 7.0;
  /// Fine, feathery leaflets (acacia, cypress).
  pub const LEAF_FINE: f32 = 8.0;
  /// Hanging moss strands.
  pub const MOSS: f32 = 9.0;
  /// Number of flora texture layers.
  pub const COUNT: u32 = 10;
  /// First foliage layer; layers at or above this are alpha-tested.
  pub const FIRST_FOLIAGE: f32 = 4.0;
}

/// One tree mesh vertex (48 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TreeVertex {
  /// Model-space position in metres, base of the trunk at the origin.
  pub position: [f32; 3],
  /// Unit normal. Foliage normals point away from the crown centre, which
  /// gives the soft, volumetric shading real canopies have.
  pub normal: [f32; 3],
  /// Texture coordinates.
  pub uv: [f32; 2],
  /// x: flora texture layer, y: wind sway weight (0 rigid .. 1 free),
  /// z: baked ambient occlusion, w: sway phase offset.
  pub params: [f32; 4],
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

type V3 = [f32; 3];

fn add(a: V3, b: V3) -> V3 {
  [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: V3, b: V3) -> V3 {
  [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: V3, s: f32) -> V3 {
  [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: V3, b: V3) -> f32 {
  a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
  [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ]
}

fn length(a: V3) -> f32 {
  dot(a, a).sqrt()
}

fn normalise(a: V3) -> V3 {
  let l = length(a);

  if l <= 1e-6 {
    [0.0, 1.0, 0.0]
  } else {
    scale(a, 1.0 / l)
  }
}

fn lerp3(a: V3, b: V3, t: f32) -> V3 {
  add(a, scale(sub(b, a), t))
}

/// Rotate `v` around unit `axis` by `angle` radians.
fn rotate(v: V3, axis: V3, angle: f32) -> V3 {
  let (s, c) = angle.sin_cos();
  add(
    add(scale(v, c), scale(cross(axis, v), s)),
    scale(axis, dot(axis, v) * (1.0 - c)),
  )
}

/// Any unit vector perpendicular to `v`.
fn perpendicular(v: V3) -> V3 {
  let helper = if v[1].abs() < 0.9 {
    [0.0, 1.0, 0.0]
  } else {
    [1.0, 0.0, 0.0]
  };
  normalise(cross(v, helper))
}

/// Direction from a yaw (around Y) and an elevation above the horizon.
fn direction(yaw: f32, elevation: f32) -> V3 {
  let (sy, cy) = yaw.sin_cos();
  let (se, ce) = elevation.sin_cos();
  [ce * sy, se, ce * cy]
}

/// Small deterministic RNG (SplitMix64).
struct Rng(u64);

impl Rng {
  fn next_u64(&mut self) -> u64 {
    self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = self.0;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
  }

  /// Uniform in `[0, 1)`.
  fn unit(&mut self) -> f32 {
    (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
  }

  /// Uniform in `[min, max)`.
  fn range(&mut self, min: f32, max: f32) -> f32 {
    min + (max - min) * self.unit()
  }
}

/// Per-vertex attributes shared by one primitive.
#[derive(Clone, Copy)]
struct Surface {
  layer: f32,
  ao: f32,
  phase: f32,
}

#[derive(Default)]
struct Builder {
  vertices: Vec<TreeVertex>,
  indices: Vec<u32>,
  /// Height used to derive trunk/branch wind weights.
  sway_height: f32,
}

impl Builder {
  fn new(sway_height: f32) -> Self {
    Self {
      sway_height,
      ..Self::default()
    }
  }

  fn wind_for(&self, position: V3, extra: f32) -> f32 {
    let h = (position[1] / self.sway_height.max(0.1)).clamp(0.0, 1.2);
    (h * h * 0.7 + extra).clamp(0.0, 1.0)
  }

  fn push(&mut self, position: V3, normal: V3, uv: [f32; 2], wind: f32, surface: Surface) -> u32 {
    let index = self.vertices.len() as u32;
    self.vertices.push(TreeVertex {
      position,
      normal: normalise(normal),
      uv,
      params: [surface.layer, wind, surface.ao, surface.phase],
    });
    index
  }

  /// A tapered generalised cylinder through `points` with per-point radii.
  ///
  /// Uses a parallel-transport frame so the tube never twists, and scales
  /// the `v` texture coordinate by the average circumference so bark
  /// texels stay roughly square along the whole length.
  #[allow(clippy::too_many_arguments)]
  fn tube(
    &mut self,
    points: &[V3],
    radii: &[f32],
    sides: u32,
    surface: Surface,
    wind_extra: f32,
    ao_base: f32,
    u_repeat: f32,
  ) {
    if points.len() < 2 || radii.len() != points.len() || sides < 3 {
      return;
    }

    let mut tangent = normalise(sub(points[1], points[0]));
    let mut normal = perpendicular(tangent);
    let mut v = 0.0;
    let average_radius = radii.iter().sum::<f32>() / radii.len() as f32;
    let v_scale = 1.0 / (std::f32::consts::TAU * average_radius.max(0.02) / u_repeat.max(0.1));
    let first = self.vertices.len() as u32;
    let count = points.len();

    for (i, point) in points.iter().enumerate() {
      let next_tangent = if i + 1 < count {
        normalise(sub(points[i + 1], *point))
      } else {
        tangent
      };
      let blended = normalise(add(tangent, next_tangent));
      // Parallel transport: rotate the previous normal onto the new
      // tangent's plane.
      normal = normalise(sub(normal, scale(blended, dot(normal, blended))));
      let binormal = cross(blended, normal);

      if i > 0 {
        v += length(sub(*point, points[i - 1])) * v_scale;
      }

      let ao = ao_base + (1.0 - ao_base) * (i as f32 / (count - 1) as f32).sqrt();

      for side in 0..=sides {
        let angle = side as f32 / sides as f32 * std::f32::consts::TAU;
        let (s, c) = angle.sin_cos();
        let offset = add(scale(normal, c), scale(binormal, s));
        let position = add(*point, scale(offset, radii[i]));
        let wind = self.wind_for(*point, wind_extra * i as f32 / (count - 1) as f32);
        self.push(
          position,
          offset,
          [side as f32 / sides as f32 * u_repeat, v],
          wind,
          Surface { ao, ..surface },
        );
      }

      tangent = next_tangent;
    }

    let ring = sides + 1;

    for i in 0..(count as u32 - 1) {
      for side in 0..sides {
        let a = first + i * ring + side;
        let b = a + 1;
        let c = a + ring;
        let d = c + 1;
        self.indices.extend_from_slice(&[a, c, b, b, c, d]);
      }
    }
  }

  /// A flat quad centred at `centre` spanning `right` and `up` (both
  /// already scaled to half extents). `normal` is the shading normal.
  #[allow(clippy::too_many_arguments)]
  fn card(
    &mut self,
    centre: V3,
    right: V3,
    up: V3,
    normal: V3,
    wind: f32,
    surface: Surface,
    uv_min: [f32; 2],
    uv_max: [f32; 2],
  ) {
    let corners = [
      (sub(sub(centre, right), up), [uv_min[0], uv_min[1]]),
      (sub(add(centre, right), up), [uv_max[0], uv_min[1]]),
      (add(add(centre, right), up), [uv_max[0], uv_max[1]]),
      (add(sub(centre, right), up), [uv_min[0], uv_max[1]]),
    ];
    let first = self.vertices.len() as u32;

    for (position, uv) in corners {
      // The lower edge of a card sways a little less than the upper edge,
      // which reads as leaves hanging from a twig.
      let edge = if uv[1] > (uv_min[1] + uv_max[1]) * 0.5 {
        1.0
      } else {
        0.8
      };
      self.push(position, normal, uv, (wind * edge).clamp(0.0, 1.0), surface);
    }

    self
      .indices
      .extend_from_slice(&[first, first + 1, first + 2, first, first + 2, first + 3]);
  }

  /// A leaf cluster: several intersecting cards around `centre`, shaded as
  /// part of a rounded crown centred at `crown_centre`.
  #[allow(clippy::too_many_arguments)]
  fn cluster(
    &mut self,
    centre: V3,
    size: f32,
    crown_centre: V3,
    crown_radius: f32,
    layer: f32,
    cards: u32,
    flatten: f32,
    rng: &mut Rng,
  ) {
    let outward = normalise(sub(centre, crown_centre));
    let depth = (length(sub(centre, crown_centre)) / crown_radius.max(0.1)).clamp(0.0, 1.0);
    let ao = 0.45 + 0.55 * depth;
    let phase = rng.unit() * std::f32::consts::TAU;
    let wind = self.wind_for(centre, 0.35);

    for card in 0..cards {
      let yaw =
        rng.unit() * std::f32::consts::PI + card as f32 * std::f32::consts::PI / cards as f32;
      let tilt =
        rng.range(-0.6, 0.6) * (1.0 - flatten) + flatten * std::f32::consts::FRAC_PI_2 * 0.92;
      let right = direction(yaw, 0.0);
      let up = rotate([0.0, 1.0, 0.0], right, tilt);
      let card_normal = cross(right, up);
      let shading = normalise(add(
        scale(outward, 0.75),
        scale(card_normal, 0.25 * card_normal[1].signum()),
      ));
      let half = size * rng.range(0.42, 0.55);
      let flip = rng.unit() > 0.5;
      let (u0, u1) = if flip { (1.0, 0.0) } else { (0.0, 1.0) };
      self.card(
        centre,
        scale(right, half),
        scale(up, half),
        shading,
        wind,
        Surface { layer, ao, phase },
        [u0, 0.0],
        [u1, 1.0],
      );
    }
  }

  /// A card aligned with a branch: the texture's `v` axis runs from the
  /// branch base to its tip, and the card is rotated `roll` radians about
  /// the branch.
  #[allow(clippy::too_many_arguments)]
  fn branch_card(
    &mut self,
    base: V3,
    tip: V3,
    width: f32,
    roll: f32,
    crown_centre: V3,
    crown_radius: f32,
    layer: f32,
    phase: f32,
  ) {
    let axis = sub(tip, base);
    let along = normalise(axis);
    let side = rotate(perpendicular(along), along, roll);
    let middle = lerp3(base, tip, 0.5);
    let outward = normalise(sub(middle, crown_centre));
    let depth = (length(sub(middle, crown_centre)) / crown_radius.max(0.1)).clamp(0.0, 1.0);
    let wind = self.wind_for(middle, 0.4);
    self.card(
      middle,
      scale(side, width * 0.5),
      scale(axis, 0.5),
      normalise(add(outward, [0.0, 0.3, 0.0])),
      wind,
      Surface {
        layer,
        ao: 0.4 + 0.6 * depth,
        phase,
      },
      [0.0, 0.0],
      [1.0, 1.0],
    );
  }

  fn finish(self) -> TreeMesh {
    let mut height: f32 = 0.0;
    let mut radius: f32 = 0.0;

    for vertex in &self.vertices {
      height = height.max(vertex.position[1]);
      radius = radius.max((vertex.position[0].powi(2) + vertex.position[2].powi(2)).sqrt());
    }

    TreeMesh {
      vertices: self.vertices,
      indices: self.indices,
      height,
      radius,
    }
  }
}

/// A curved growth path: starts at `start` heading `dir`, bends towards
/// `bend` (a direction) by `bend_amount` over its `length`, with a little
/// random wobble.
#[allow(clippy::too_many_arguments)]
fn grow_path(
  start: V3,
  dir: V3,
  bend: V3,
  bend_amount: f32,
  length: f32,
  segments: usize,
  wobble: f32,
  rng: &mut Rng,
) -> Vec<V3> {
  let mut points = Vec::with_capacity(segments + 1);
  let mut position = start;
  let mut heading = normalise(dir);
  let step = length / segments as f32;
  points.push(position);

  for _ in 0..segments {
    heading = normalise(add(
      add(
        heading,
        scale(normalise(bend), bend_amount / segments as f32),
      ),
      [
        rng.range(-wobble, wobble),
        rng.range(-wobble, wobble) * 0.5,
        rng.range(-wobble, wobble),
      ],
    ));
    position = add(position, scale(heading, step));
    points.push(position);
  }

  points
}

fn taper(count: usize, from: f32, to: f32, power: f32) -> Vec<f32> {
  (0..count)
    .map(|i| {
      let t = i as f32 / (count - 1).max(1) as f32;
      from + (to - from) * t.powf(power)
    })
    .collect()
}

fn bark(layer: f32) -> Surface {
  Surface {
    layer,
    ao: 1.0,
    phase: 0.0,
  }
}

fn build_oak() -> TreeMesh {
  let mut rng = Rng(0x0a4_71ee);
  let mut b = Builder::new(16.0);
  let trunk_height = 5.2;
  let lean = direction(rng.unit() * TAU, 1.45);
  let trunk = grow_path(
    [0.0, -0.3, 0.0],
    [0.0, 1.0, 0.0],
    lean,
    0.25,
    trunk_height,
    8,
    0.03,
    &mut rng,
  );
  let mut radii = taper(trunk.len(), 0.62, 0.36, 0.7);
  radii[0] = 0.95;
  radii[1] = 0.7;
  b.tube(&trunk, &radii, 9, bark(layers::BARK_OAK), 0.0, 0.55, 2.0);

  let crown_centre = [0.0, 11.0, 0.0];
  let crown_radius = 7.5;
  let top = trunk[trunk.len() - 1];
  let limbs = 4;
  let base_yaw = rng.unit() * TAU;

  for limb in 0..limbs {
    let yaw = base_yaw + limb as f32 / limbs as f32 * TAU + rng.range(-0.35, 0.35);
    let elevation = rng.range(0.55, 0.95);
    let limb_length = rng.range(5.5, 7.2);
    let start = lerp3(trunk[trunk.len() - 3], top, rng.range(0.0, 1.0));
    let path = grow_path(
      start,
      direction(yaw, elevation),
      [0.0, 1.0, 0.0],
      0.5,
      limb_length,
      6,
      0.08,
      &mut rng,
    );
    let radii = taper(path.len(), 0.3, 0.07, 0.8);
    b.tube(&path, &radii, 6, bark(layers::BARK_OAK), 0.25, 0.6, 1.0);

    for branch in 0..4 {
      let t = 0.35 + branch as f32 * 0.18 + rng.range(-0.05, 0.05);
      let index = ((path.len() - 1) as f32 * t) as usize;
      let origin = path[index.min(path.len() - 1)];
      let side_yaw = yaw + if branch % 2 == 0 { 1.0 } else { -1.0 } * rng.range(0.6, 1.3);
      let branch_path = grow_path(
        origin,
        direction(side_yaw, rng.range(0.1, 0.7)),
        [0.0, 1.0, 0.0],
        0.4,
        rng.range(2.4, 3.8),
        4,
        0.12,
        &mut rng,
      );
      let radii = taper(branch_path.len(), 0.09, 0.025, 1.0);
      b.tube(
        &branch_path,
        &radii,
        4,
        bark(layers::BARK_OAK),
        0.45,
        0.7,
        1.0,
      );

      for (k, point) in branch_path.iter().enumerate().skip(2) {
        let jitter = [
          rng.range(-0.6, 0.6),
          rng.range(-0.3, 0.6),
          rng.range(-0.6, 0.6),
        ];
        let size = if k == branch_path.len() - 1 { 3.4 } else { 2.8 };
        b.cluster(
          add(*point, jitter),
          size,
          crown_centre,
          crown_radius,
          layers::LEAF_BROAD,
          3,
          0.0,
          &mut rng,
        );
      }
    }

    let tip = path[path.len() - 1];
    b.cluster(
      tip,
      3.6,
      crown_centre,
      crown_radius,
      layers::LEAF_BROAD,
      3,
      0.0,
      &mut rng,
    );
  }

  // Inner fill so the crown does not look hollow from below.
  for _ in 0..10 {
    let centre = add(
      crown_centre,
      [
        rng.range(-4.0, 4.0),
        rng.range(-1.5, 3.0),
        rng.range(-4.0, 4.0),
      ],
    );
    b.cluster(
      centre,
      3.4,
      crown_centre,
      crown_radius,
      layers::LEAF_BROAD,
      3,
      0.0,
      &mut rng,
    );
  }

  b.finish()
}

fn build_pine() -> TreeMesh {
  let mut rng = Rng(0x0b1_9e5);
  let mut b = Builder::new(22.0);
  let height = 22.0;
  let trunk = grow_path(
    [0.0, -0.3, 0.0],
    [0.0, 1.0, 0.0],
    [0.3, 1.0, 0.0],
    0.08,
    height,
    12,
    0.015,
    &mut rng,
  );
  let mut radii = taper(trunk.len(), 0.4, 0.04, 0.9);
  radii[0] = 0.55;
  b.tube(&trunk, &radii, 8, bark(layers::BARK_PINE), 0.0, 0.6, 2.0);

  let crown_centre = [0.0, 17.5, 0.0];
  let crown_radius = 5.0;
  let whorls = 9;

  for whorl in 0..whorls {
    let t = whorl as f32 / (whorls - 1) as f32;
    let y = 10.5 + t * 10.5;
    let count = 4 + (rng.unit() * 2.0) as usize;
    let whorl_yaw = rng.unit() * TAU;
    let origin_index = ((y / height) * (trunk.len() - 1) as f32) as usize;
    let origin = trunk[origin_index.min(trunk.len() - 1)];

    for branch in 0..count {
      let yaw = whorl_yaw + branch as f32 / count as f32 * TAU + rng.range(-0.3, 0.3);
      let branch_length = (4.2 * (1.0 - t * 0.75)) * rng.range(0.8, 1.15);
      let path = grow_path(
        [origin[0], y, origin[2]],
        direction(yaw, rng.range(0.05, 0.35)),
        [0.0, 1.0, 0.0],
        0.5,
        branch_length,
        3,
        0.08,
        &mut rng,
      );
      let radii = taper(path.len(), 0.08, 0.02, 1.0);
      b.tube(&path, &radii, 3, bark(layers::BARK_PINE), 0.4, 0.6, 1.0);
      let phase = rng.unit() * TAU;

      // Needle sprays run along the outer part of the branch and curl up
      // at the tip, the way pine foliage clumps at branch ends.
      let start = lerp3(path[0], path[path.len() - 1], 0.3);
      let tip = add(path[path.len() - 1], [0.0, 0.5, 0.0]);

      for roll in [0.0f32, 1.1, 2.2] {
        b.branch_card(
          start,
          tip,
          branch_length * 0.75 + 0.8,
          roll + rng.range(-0.2, 0.2),
          crown_centre,
          crown_radius,
          layers::NEEDLES,
          phase,
        );
      }
    }
  }

  // Leading shoot tuft.
  let crown_top = trunk[trunk.len() - 1];
  b.cluster(
    add(crown_top, [0.0, -0.4, 0.0]),
    2.6,
    crown_centre,
    crown_radius,
    layers::NEEDLES,
    3,
    0.0,
    &mut rng,
  );

  b.finish()
}

fn build_spruce() -> TreeMesh {
  let mut rng = Rng(0x05b_12ce);
  let mut b = Builder::new(20.0);
  let height = 20.0;
  let trunk = grow_path(
    [0.0, -0.3, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 1.0, 0.0],
    0.0,
    height,
    10,
    0.005,
    &mut rng,
  );
  let mut radii = taper(trunk.len(), 0.38, 0.03, 1.0);
  radii[0] = 0.5;
  b.tube(&trunk, &radii, 7, bark(layers::BARK_PINE), 0.0, 0.5, 2.0);

  let crown_centre = [0.0, 9.0, 0.0];
  let crown_radius = 5.0;
  let whorls = 24;

  for whorl in 0..whorls {
    let t = whorl as f32 / (whorls - 1) as f32;
    let y = 1.3 + t * (height - 1.8);
    let count = if t > 0.85 { 4 } else { 6 };
    let whorl_yaw = rng.unit() * TAU;
    let branch_length = (0.35 + (1.0 - t).powf(1.1) * 4.4) * rng.range(0.9, 1.1);

    for branch in 0..count {
      let yaw = whorl_yaw + branch as f32 / count as f32 * TAU + rng.range(-0.2, 0.2);
      // Branches droop, then lift slightly at the tip.
      let droop = -0.35 + t * 0.35;
      let base = [0.0, y, 0.0];
      let dir = direction(yaw, droop);
      let tip = add(
        add(base, scale(dir, branch_length)),
        [0.0, branch_length * 0.12, 0.0],
      );
      let phase = rng.unit() * TAU;
      b.branch_card(
        base,
        tip,
        branch_length * 0.7 + 0.5,
        0.0,
        crown_centre,
        crown_radius,
        layers::NEEDLES,
        phase,
      );
      b.branch_card(
        base,
        tip,
        branch_length * 0.55 + 0.4,
        FRAC_PI_2,
        crown_centre,
        crown_radius,
        layers::NEEDLES,
        phase,
      );
    }
  }

  b.cluster(
    [0.0, height - 0.4, 0.0],
    1.6,
    crown_centre,
    crown_radius,
    layers::NEEDLES,
    2,
    0.0,
    &mut rng,
  );
  b.finish()
}

fn build_palm() -> TreeMesh {
  let mut rng = Rng(0x0ba_1111);
  let mut b = Builder::new(12.0);
  let lean_yaw = rng.unit() * TAU;
  // A gentle S-curve: lean out, then curve back up towards the light.
  let trunk = grow_path(
    [0.0, -0.2, 0.0],
    direction(lean_yaw, 1.2),
    [0.0, 1.0, 0.0],
    0.55,
    12.0,
    16,
    0.01,
    &mut rng,
  );
  let mut radii = taper(trunk.len(), 0.24, 0.17, 0.6);
  radii[0] = 0.36;
  radii[1] = 0.27;
  b.tube(&trunk, &radii, 8, bark(layers::BARK_PALM), 0.0, 0.6, 1.0);

  let top = trunk[trunk.len() - 1];
  let crown_centre = add(top, [0.0, -0.5, 0.0]);
  let fronds = 15;

  for frond in 0..fronds {
    let young = frond >= 12;
    let yaw = frond as f32 / 12.0 * TAU + rng.range(-0.2, 0.2) + if young { 0.5 } else { 0.0 };
    let elevation = if young {
      rng.range(0.9, 1.2)
    } else {
      rng.range(0.15, 0.65)
    };
    let frond_length = if young {
      rng.range(2.5, 3.2)
    } else {
      rng.range(4.6, 5.6)
    };
    let segments = 9;
    let heading = direction(yaw, elevation);
    let side = normalise(cross([0.0, 1.0, 0.0], heading));
    let phase = rng.unit() * TAU;
    let mut previous: Option<(u32, u32, u32)> = None;

    for s in 0..=segments {
      let t = s as f32 / segments as f32;
      // Arching droop: the rib bends down increasingly towards the tip.
      let droop = if young { 0.3 } else { 1.9 } * t * t;
      let along = scale(heading, frond_length * t);
      let rib = add(add(top, along), [0.0, -droop * frond_length * 0.35, 0.0]);
      let width = (if t < 0.12 {
        t / 0.12 * 0.9
      } else {
        1.0 - (t - 0.12) * 0.75
      }) * 0.75;
      // Leaflets fold up from the rib in a shallow V.
      let fold = [0.0, width * 0.35, 0.0];
      let left = add(add(rib, scale(side, -width)), fold);
      let right = add(add(rib, scale(side, width)), fold);
      let normal = normalise(add(normalise(sub(rib, crown_centre)), [0.0, 1.2, 0.0]));
      let wind = b.wind_for(rib, 0.3 + t * 0.7);
      let surface = Surface {
        layer: layers::PALM_FROND,
        ao: 0.55 + 0.45 * t,
        phase,
      };
      let l = b.push(left, normal, [0.0, t], wind, surface);
      let r_mid = b.push(rib, normal, [0.5, t], wind, surface);
      let r = b.push(right, normal, [1.0, t], wind, surface);

      if let Some((pl, pm, pr)) = previous {
        b.indices
          .extend_from_slice(&[pl, l, pm, pm, l, r_mid, pm, r_mid, pr, pr, r_mid, r]);
      }

      previous = Some((l, r_mid, r));
    }
  }

  // Coconuts.
  for nut in 0..4 {
    let yaw = nut as f32 * 1.7 + 0.3;
    let centre = add(
      top,
      add(scale(direction(yaw, 0.0), 0.35), [0.0, -0.45, 0.0]),
    );
    let path = [
      add(centre, [0.0, -0.14, 0.0]),
      centre,
      add(centre, [0.0, 0.14, 0.0]),
    ];
    b.tube(
      &path,
      &[0.04, 0.16, 0.04],
      5,
      Surface {
        layer: layers::BARK_PALM,
        ao: 0.6,
        phase: 0.0,
      },
      0.0,
      0.6,
      1.0,
    );
  }

  b.finish()
}

fn build_jungle() -> TreeMesh {
  let mut rng = Rng(0x01f_9e1e);
  let mut b = Builder::new(30.0);
  let trunk = grow_path(
    [0.0, -0.3, 0.0],
    [0.0, 1.0, 0.0],
    [0.2, 1.0, 0.1],
    0.1,
    22.0,
    12,
    0.012,
    &mut rng,
  );
  let mut radii = taper(trunk.len(), 0.72, 0.4, 0.8);
  radii[0] = 1.1;
  b.tube(&trunk, &radii, 10, bark(layers::BARK_SMOOTH), 0.0, 0.5, 3.0);

  // Plank buttress roots.
  let buttresses = 5;
  let base_yaw = rng.unit() * TAU;

  for root in 0..buttresses {
    let yaw = base_yaw + root as f32 / buttresses as f32 * TAU + rng.range(-0.25, 0.25);
    let out = direction(yaw, 0.0);
    let reach = rng.range(2.2, 3.2);
    let rise = rng.range(2.8, 4.0);
    let side = normalise(cross([0.0, 1.0, 0.0], out));
    let surface = Surface {
      layer: layers::BARK_SMOOTH,
      ao: 0.55,
      phase: 0.0,
    };
    let inner_top = [out[0] * 0.5, rise, out[2] * 0.5];
    let inner_bottom = [out[0] * 0.5, -0.3, out[2] * 0.5];
    let outer = [out[0] * (0.5 + reach), -0.3, out[2] * (0.5 + reach)];
    let mid = [
      out[0] * (0.5 + reach * 0.35),
      rise * 0.35,
      out[2] * (0.5 + reach * 0.35),
    ];

    for flip in [1.0f32, -1.0] {
      let offset = scale(side, 0.09 * flip);
      let normal = scale(side, flip);
      let a = b.push(add(inner_bottom, offset), normal, [0.0, 0.0], 0.0, surface);
      let c = b.push(add(outer, offset), normal, [1.0, 0.0], 0.0, surface);
      let d = b.push(add(mid, offset), normal, [0.6, 0.5], 0.0, surface);
      let e = b.push(add(inner_top, offset), normal, [0.0, 1.0], 0.0, surface);

      if flip > 0.0 {
        b.indices.extend_from_slice(&[a, c, d, a, d, e]);
      } else {
        b.indices.extend_from_slice(&[a, d, c, a, e, d]);
      }
    }
  }

  let crown_centre = [0.0, 27.0, 0.0];
  let crown_radius = 10.0;
  let top = trunk[trunk.len() - 1];
  let limbs = 6;

  for limb in 0..limbs {
    let yaw = limb as f32 / limbs as f32 * TAU + rng.range(-0.3, 0.3);
    let start = lerp3(trunk[trunk.len() - 3], top, rng.unit());
    let path = grow_path(
      start,
      direction(yaw, rng.range(0.45, 0.8)),
      [0.0, 1.0, 0.0],
      0.15,
      rng.range(6.5, 9.0),
      5,
      0.06,
      &mut rng,
    );
    let radii = taper(path.len(), 0.32, 0.08, 0.9);
    b.tube(&path, &radii, 6, bark(layers::BARK_SMOOTH), 0.2, 0.6, 1.0);

    for (k, point) in path.iter().enumerate().skip(2) {
      for _ in 0..2 {
        let jitter = [
          rng.range(-1.6, 1.6),
          rng.range(-0.2, 1.4),
          rng.range(-1.6, 1.6),
        ];
        b.cluster(
          add(*point, jitter),
          if k == path.len() - 1 { 4.2 } else { 3.6 },
          crown_centre,
          crown_radius,
          layers::LEAF_TROPICAL,
          3,
          0.35,
          &mut rng,
        );
      }
    }
  }

  // Top cap of the umbrella crown.
  for _ in 0..8 {
    let centre = add(
      top,
      [
        rng.range(-4.0, 4.0),
        rng.range(2.5, 5.0),
        rng.range(-4.0, 4.0),
      ],
    );
    b.cluster(
      centre,
      4.4,
      crown_centre,
      crown_radius,
      layers::LEAF_TROPICAL,
      3,
      0.5,
      &mut rng,
    );
  }

  // Hanging lianas.
  for _ in 0..6 {
    let yaw = rng.unit() * TAU;
    let anchor = add(
      top,
      add(
        scale(direction(yaw, 0.0), rng.range(2.0, 6.0)),
        [0.0, rng.range(-0.5, 1.5), 0.0],
      ),
    );
    let hang = rng.range(5.0, 9.0);
    let right = scale(direction(yaw + FRAC_PI_2, 0.0), 0.5);
    let wind = b.wind_for(anchor, 0.6);
    b.card(
      add(anchor, [0.0, -hang * 0.5, 0.0]),
      right,
      [0.0, hang * 0.5, 0.0],
      direction(yaw, 0.0),
      wind,
      Surface {
        layer: layers::MOSS,
        ao: 0.6,
        phase: rng.unit() * TAU,
      },
      [0.0, 0.0],
      [1.0, 1.0],
    );
  }

  b.finish()
}

fn build_cypress() -> TreeMesh {
  let mut rng = Rng(0x0c1_9e55);
  let mut b = Builder::new(19.0);
  let trunk = grow_path(
    [0.0, -0.4, 0.0],
    [0.0, 1.0, 0.0],
    [0.1, 1.0, 0.0],
    0.05,
    18.5,
    12,
    0.012,
    &mut rng,
  );
  let mut radii = taper(trunk.len(), 0.46, 0.06, 0.8);
  // Strongly flared, fluted base typical of bald cypress in standing water.
  radii[0] = 1.35;
  radii[1] = 0.8;
  radii[2] = 0.55;
  b.tube(&trunk, &radii, 10, bark(layers::BARK_SMOOTH), 0.0, 0.5, 3.0);

  // Cypress "knees" poking up around the base.
  for knee in 0..6 {
    let yaw = knee as f32 * 1.05 + rng.range(-0.3, 0.3);
    let base = scale(direction(yaw, 0.0), rng.range(1.8, 3.2));
    let path = [base, add(base, [0.0, rng.range(0.4, 0.8), 0.0])];
    b.tube(
      &path,
      &[0.14, 0.03],
      5,
      bark(layers::BARK_SMOOTH),
      0.0,
      0.5,
      1.0,
    );
  }

  let crown_centre = [0.0, 14.0, 0.0];
  let crown_radius = 5.5;

  for level in 0..9 {
    let t = level as f32 / 8.0;
    let y = 7.0 + t * 11.0;
    let origin_index = ((y / 18.5) * (trunk.len() - 1) as f32) as usize;
    let origin = trunk[origin_index.min(trunk.len() - 1)];

    for _ in 0..3 {
      let yaw = rng.unit() * TAU;
      let branch_length = (4.8 - t * 3.0) * rng.range(0.8, 1.15);
      let path = grow_path(
        [origin[0], y, origin[2]],
        direction(yaw, rng.range(-0.05, 0.25)),
        [0.0, 1.0, 0.0],
        0.3,
        branch_length,
        3,
        0.1,
        &mut rng,
      );
      let radii = taper(path.len(), 0.1, 0.03, 1.0);
      b.tube(&path, &radii, 4, bark(layers::BARK_SMOOTH), 0.4, 0.6, 1.0);

      for point in path.iter().skip(1) {
        b.cluster(
          add(*point, [0.0, 0.3, 0.0]),
          2.6,
          crown_centre,
          crown_radius,
          layers::LEAF_FINE,
          2,
          0.55,
          &mut rng,
        );
      }

      // Spanish moss drapes from the lower branches.
      if t < 0.6 && rng.unit() < 0.8 {
        let anchor = path[path.len() / 2 + 1];
        let hang = rng.range(1.6, 3.2);
        let right = scale(direction(yaw + FRAC_PI_2, 0.0), 0.45);
        let wind = b.wind_for(anchor, 0.7);
        b.card(
          add(anchor, [0.0, -hang * 0.5, 0.0]),
          right,
          [0.0, hang * 0.5, 0.0],
          direction(yaw, 0.0),
          wind,
          Surface {
            layer: layers::MOSS,
            ao: 0.7,
            phase: rng.unit() * TAU,
          },
          [0.0, 0.0],
          [1.0, 1.0],
        );
      }
    }
  }

  b.finish()
}

fn build_acacia() -> TreeMesh {
  let mut rng = Rng(0x0ac_ac1a);
  let mut b = Builder::new(8.0);
  let trunk = grow_path(
    [0.0, -0.2, 0.0],
    [0.05, 1.0, 0.0],
    [0.4, 1.0, 0.0],
    0.2,
    2.4,
    4,
    0.03,
    &mut rng,
  );
  let mut radii = taper(trunk.len(), 0.3, 0.22, 1.0);
  radii[0] = 0.4;
  b.tube(&trunk, &radii, 7, bark(layers::BARK_OAK), 0.0, 0.6, 1.5);

  let crown_centre = [0.0, 7.2, 0.0];
  let crown_radius = 5.5;
  let fork = trunk[trunk.len() - 1];
  let base_yaw = rng.unit() * TAU;

  for limb in 0..3 {
    let yaw = base_yaw + limb as f32 * 2.1 + rng.range(-0.3, 0.3);
    let path = grow_path(
      fork,
      direction(yaw, rng.range(0.85, 1.1)),
      [0.0, 1.0, 0.0],
      0.1,
      rng.range(4.2, 5.0),
      5,
      0.05,
      &mut rng,
    );
    let radii = taper(path.len(), 0.2, 0.06, 1.0);
    b.tube(&path, &radii, 5, bark(layers::BARK_OAK), 0.2, 0.6, 1.0);
    let end = path[path.len() - 1];

    // Spreading twigs carrying a flat, layered canopy.
    for twig in 0..4 {
      let twig_yaw = yaw + (twig as f32 - 1.5) * 0.9;
      let twig_path = grow_path(
        end,
        direction(twig_yaw, rng.range(0.05, 0.3)),
        [0.0, 0.0, 0.0],
        0.0,
        rng.range(2.0, 3.2),
        2,
        0.1,
        &mut rng,
      );
      let radii = taper(twig_path.len(), 0.06, 0.02, 1.0);
      b.tube(&twig_path, &radii, 3, bark(layers::BARK_OAK), 0.4, 0.7, 1.0);

      for point in twig_path.iter().skip(1) {
        let centre = add(
          *point,
          [
            rng.range(-0.5, 0.5),
            rng.range(0.1, 0.5),
            rng.range(-0.5, 0.5),
          ],
        );
        b.cluster(
          centre,
          3.0,
          crown_centre,
          crown_radius,
          layers::LEAF_FINE,
          2,
          0.9,
          &mut rng,
        );
      }
    }
  }

  b.finish()
}

fn build_shrub() -> TreeMesh {
  let mut rng = Rng(0x05_4b0b);
  let mut b = Builder::new(2.2);
  let crown_centre = [0.0, 1.0, 0.0];
  let crown_radius = 1.4;

  for stem in 0..6 {
    let yaw = stem as f32 * 1.05 + rng.range(-0.3, 0.3);
    let path = grow_path(
      [0.0, -0.1, 0.0],
      direction(yaw, rng.range(0.9, 1.3)),
      [0.0, 1.0, 0.0],
      0.2,
      rng.range(1.3, 1.9),
      3,
      0.1,
      &mut rng,
    );
    let radii = taper(path.len(), 0.04, 0.012, 1.0);
    b.tube(&path, &radii, 3, bark(layers::BARK_OAK), 0.4, 0.5, 1.0);

    for point in path.iter().skip(1) {
      b.cluster(
        *point,
        1.25,
        crown_centre,
        crown_radius,
        layers::LEAF_BROAD,
        3,
        0.0,
        &mut rng,
      );
    }
  }

  b.finish()
}

/// Build the mesh for one species. Deterministic: the same species always
/// produces exactly the same mesh.
pub fn build_species_mesh(species: TreeSpecies) -> TreeMesh {
  match species {
    TreeSpecies::Oak => build_oak(),
    TreeSpecies::Pine => build_pine(),
    TreeSpecies::Spruce => build_spruce(),
    TreeSpecies::Palm => build_palm(),
    TreeSpecies::Jungle => build_jungle(),
    TreeSpecies::Cypress => build_cypress(),
    TreeSpecies::Acacia => build_acacia(),
    TreeSpecies::Shrub => build_shrub(),
  }
}

/// All species meshes merged into one vertex/index buffer, with a
/// per-species `(first_index, index_count, base_vertex)` range.
pub struct TreeLibrary {
  /// Merged vertices.
  pub vertices: Vec<TreeVertex>,
  /// Merged indices (relative to each species' base vertex).
  pub indices: Vec<u32>,
  /// `(first_index, index_count, base_vertex)` per species.
  pub ranges: [(u32, u32, i32); SPECIES_COUNT],
  /// `(height, radius)` per species in metres.
  pub bounds: [(f32, f32); SPECIES_COUNT],
}

/// Build every species and merge them.
pub fn build_tree_library() -> TreeLibrary {
  let meshes: Vec<TreeMesh> = TreeSpecies::ALL
    .iter()
    .map(|species| build_species_mesh(*species))
    .collect();
  merge_tree_meshes(&meshes)
}

/// Merge one mesh per species slot into a library.
pub fn merge_tree_meshes(meshes: &[TreeMesh]) -> TreeLibrary {
  let mut vertices = Vec::new();
  let mut indices = Vec::new();
  let mut ranges = [(0, 0, 0); SPECIES_COUNT];
  let mut bounds = [(1.0, 1.0); SPECIES_COUNT];

  for (slot, mesh) in meshes.iter().take(SPECIES_COUNT).enumerate() {
    ranges[slot] = (
      indices.len() as u32,
      mesh.indices.len() as u32,
      vertices.len() as i32,
    );
    bounds[slot] = (mesh.height.max(0.5), mesh.radius.max(0.25));
    vertices.extend_from_slice(&mesh.vertices);
    indices.extend_from_slice(&mesh.indices);
  }

  TreeLibrary {
    vertices,
    indices,
    ranges,
    bounds,
  }
}

/// Largest custom tree mesh accepted, in vertices.
pub const MAX_CUSTOM_TREE_VERTICES: usize = 65_536;

/// Build a tree mesh from host-supplied arrays, validating everything.
///
/// `positions` and `normals` hold three floats per vertex, `uvs` two, and
/// `indices` three per triangle. `layers` optionally gives a flora texture
/// layer per vertex (default 0, opaque bark); layers at or above
/// [`layers::FIRST_FOLIAGE`] are alpha-tested. `wind` optionally gives a
/// sway weight from 0 (rigid) to 1 per vertex; by default it grows with
/// height. Units are metres with the base of the trunk at the origin.
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
      "a tree model may have at most {MAX_CUSTOM_TREE_VERTICES} vertices."
    ));
  }

  if normals.len() != count * 3 {
    return Err("normals must hold three numbers per vertex.".to_string());
  }

  if uvs.len() != count * 2 {
    return Err("uvs must hold two numbers per vertex.".to_string());
  }

  if indices.is_empty() || !indices.len().is_multiple_of(3) {
    return Err("indices must hold three indices per triangle.".to_string());
  }

  if indices.iter().any(|index| *index as usize >= count) {
    return Err("indices must refer to existing vertices.".to_string());
  }

  if positions
    .iter()
    .chain(normals)
    .chain(uvs)
    .any(|value| !value.is_finite())
  {
    return Err("positions, normals, and uvs must be finite.".to_string());
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
  let mut builder = Builder::new(height);

  for i in 0..count {
    let position = [positions[i * 3], positions[i * 3 + 1], positions[i * 3 + 2]];
    let layer = texture_layers.map_or(0.0, |layers| layers[i].floor());
    let sway = wind.map_or_else(|| builder.wind_for(position, 0.0), |weights| weights[i]);
    builder.push(
      position,
      [normals[i * 3], normals[i * 3 + 1], normals[i * 3 + 2]],
      [uvs[i * 2], uvs[i * 2 + 1]],
      sway,
      Surface {
        layer,
        ao: 1.0,
        phase: (i as f32 * 0.618).fract() * TAU,
      },
    );
  }

  builder.indices.extend_from_slice(indices);
  Ok(builder.finish())
}

const _: () = assert!(std::mem::size_of::<TreeVertex>() == 48);

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn every_species_builds_a_valid_mesh() {
    for species in TreeSpecies::ALL {
      let mesh = build_species_mesh(species);

      assert!(!mesh.vertices.is_empty(), "{species:?} has no vertices");
      assert_eq!(mesh.indices.len() % 3, 0);
      assert!(mesh
        .indices
        .iter()
        .all(|index| (*index as usize) < mesh.vertices.len()));
      assert!(mesh.vertices.iter().all(|vertex| vertex
        .position
        .iter()
        .chain(vertex.normal.iter())
        .chain(vertex.uv.iter())
        .chain(vertex.params.iter())
        .all(|value| value.is_finite())));
      assert!(mesh.height > 1.0);
    }
  }

  #[test]
  fn meshes_are_deterministic() {
    for species in TreeSpecies::ALL {
      assert_eq!(build_species_mesh(species), build_species_mesh(species));
    }
  }

  #[test]
  fn triangle_budgets_stay_real_time_friendly() {
    for species in TreeSpecies::ALL {
      let triangles = build_species_mesh(species).indices.len() / 3;
      assert!(triangles < 4_000, "{species:?} has {triangles} triangles");
    }
  }

  #[test]
  fn species_have_their_characteristic_proportions() {
    let palm = build_species_mesh(TreeSpecies::Palm);
    let spruce = build_species_mesh(TreeSpecies::Spruce);
    let acacia = build_species_mesh(TreeSpecies::Acacia);
    let shrub = build_species_mesh(TreeSpecies::Shrub);
    let jungle = build_species_mesh(TreeSpecies::Jungle);

    // Spruce is tall and narrow, acacia is short and wide.
    assert!(spruce.height / spruce.radius > 3.0);
    assert!(acacia.radius / acacia.height > 0.6);
    assert!(shrub.height < 3.0);
    assert!(jungle.height > palm.height);
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
      Some(&[0.0, 12.0, 0.0]),
      None
    )
    .is_err());
    assert!(mesh_from_arrays(&[f32::NAN; 9], &normals, &uvs, &[0, 1, 2], None, None).is_err());
  }

  #[test]
  fn library_ranges_cover_all_indices() {
    let library = build_tree_library();
    let total: u32 = library.ranges.iter().map(|range| range.1).sum();

    assert_eq!(total as usize, library.indices.len());
  }
}
