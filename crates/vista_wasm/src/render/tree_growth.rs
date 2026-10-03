//! Tree growth by space colonisation.
//!
//! Every procedural tree is grown when the renderer starts, following
//! Runions, Lane and Prusinkiewicz (2007): attraction points scattered
//! through a species' crown envelope pull the nearest branch node towards
//! them, nodes grow a step towards the points pulling them, and points
//! are used up once a branch reaches them. Branches therefore fill the
//! crown without crossing, leave gaps where the envelope's lobes leave
//! no points, and fork where points pull a node two ways.
//!
//! Radii follow the pipe model: at every fork `r_parent^2.5` is the sum
//! of the children's `r^2.5`, and along a branch each node adds the pipes
//! of the foliage it carries, so limbs taper towards their twigs. The
//! radii are scaled so the trunk at breast height matches the species'
//! trunk radius in `render/flora.rs`, which also sets how wide its roots
//! reach when grounding.
//!
//! Branches are meshed as generalised cylinders along their node paths
//! with parallel-transport frames, and foliage as clump cards at twig
//! tips, textured with the leaf atlas (`shaders/texture_gen.wgsl`). Each
//! vertex carries the data the layered wind in `shaders/trees.wgsl`
//! needs: its branch's pivot, its branch level, its stiffness and a
//! phase.
//!
//! Growth is sequential, so it runs here, in WASM, rather than on the
//! GPU. It is deterministic: a species, variant and age class always grow
//! the same tree.

use crate::render::flora::{species_trunk_radius, TRUNK_BREAST_HEIGHT};
use crate::render::tree_models::{layers, pack_normal, TreeMesh, TreeSpecies, TreeVertex};

/// Distinct grown shapes per species at most (`FloraOptions::variantsPerSpecies`).
pub const VARIANTS: usize = 4;
/// Age classes per variant: young, mature, old, and the wind-shaped
/// krummholz of stunted trees.
pub const AGES: usize = 4;
/// Levels of detail per mesh: the full mesh, and the lighter one drawn
/// from 50 m.
pub const LODS: usize = 2;

/// A tree's age class, stored in bits 12 and 13 of its species word. 0
/// is mature, so a hand-placed tree, whose bits are 0, is mature.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Age {
  /// A mature tree.
  Mature = 0,
  /// A young tree: a narrower, lower crown with fewer branches. Shrubs
  /// grow a dwarf, prostrate form.
  Young = 1,
  /// An old tree: broader and more open, with heavier, drooping limbs.
  Old = 2,
  /// A stunted tree shaped by the wind: branches die back on the
  /// windward side (+x in model space) and the crown streams leeward.
  Krummholz = 3,
}

impl Age {
  /// Every age class in index order.
  pub const ALL: [Age; AGES] = [Age::Mature, Age::Young, Age::Old, Age::Krummholz];
}

pub(crate) type V3 = [f32; 3];

pub(crate) fn add(a: V3, b: V3) -> V3 {
  [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub(a: V3, b: V3) -> V3 {
  [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale(a: V3, s: f32) -> V3 {
  [a[0] * s, a[1] * s, a[2] * s]
}

pub(crate) fn dot(a: V3, b: V3) -> f32 {
  a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cross(a: V3, b: V3) -> V3 {
  [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ]
}

pub(crate) fn length(a: V3) -> f32 {
  dot(a, a).sqrt()
}

pub(crate) fn normalise(a: V3) -> V3 {
  let l = length(a);

  if l <= 1e-6 {
    [0.0, 1.0, 0.0]
  } else {
    scale(a, 1.0 / l)
  }
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

/// Small deterministic RNG (SplitMix64).
pub(crate) struct Rng(pub u64);

impl Rng {
  fn next_u64(&mut self) -> u64 {
    self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = self.0;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
  }

  /// Uniform in `[0, 1)`.
  pub fn unit(&mut self) -> f32 {
    (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
  }

  /// Uniform in `[min, max)`.
  pub fn range(&mut self, min: f32, max: f32) -> f32 {
    min + (max - min) * self.unit()
  }

  /// A point in the unit ball.
  fn ball(&mut self) -> V3 {
    loop {
      let p = [
        self.range(-1.0, 1.0),
        self.range(-1.0, 1.0),
        self.range(-1.0, 1.0),
      ];

      if dot(p, p) <= 1.0 {
        return p;
      }
    }
  }
}

/// How a crown's radius changes with height through its envelope.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Profile {
  /// An ellipsoid (flattened when its radius exceeds its half height).
  Ellipsoid,
  /// A narrow cone, widest at the bottom.
  Cone,
  /// Flat on top and widest just below it, curving in underneath.
  Umbrella,
  /// A column with rounded ends.
  Column,
  /// A dome, widest at the ground.
  Dome,
}

/// The crown envelope attraction points are scattered in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Envelope {
  /// Bottom and top of the crown in metres.
  pub bottom: f32,
  /// See `bottom`.
  pub top: f32,
  /// Widest radius in metres.
  pub radius: f32,
  /// Offset of the crown's axis along +x in metres (leeward is -x).
  pub shift: f32,
  /// Its shape.
  pub profile: Profile,
}

impl Envelope {
  /// The crown's radius at `height` metres, 0 outside it.
  pub fn radius_at(&self, height: f32) -> f32 {
    let span = (self.top - self.bottom).max(1e-3);
    let u = (height - self.bottom) / span;

    if !(0.0..=1.0).contains(&u) {
      return 0.0;
    }

    let shape = match self.profile {
      Profile::Ellipsoid => (1.0 - (2.0 * u - 1.0).powi(2)).max(0.0).sqrt(),
      Profile::Cone => (1.0 - u).powf(0.9) * (u * 12.0).min(1.0).sqrt(),
      Profile::Umbrella => {
        (1.0 - (1.0 - u).powi(2)).max(0.0).sqrt() * (1.0 - 0.35 * ((u - 0.85) / 0.15).max(0.0))
      }
      Profile::Column => (1.0 - (2.0 * u - 1.0).abs().powi(4)).max(0.0).powf(0.25),
      Profile::Dome => (1.0 - u * u).max(0.0).sqrt(),
    };

    self.radius * shape
  }

  /// Whether `p` lies inside the envelope scaled by `grow` about its
  /// centre.
  pub fn contains(&self, p: V3, grow: f32) -> bool {
    let middle = (self.bottom + self.top) * 0.5;
    let shrunk = [
      (p[0] - self.shift) / grow,
      middle + (p[1] - middle) / grow,
      p[2] / grow,
    ];
    let reach = self.radius_at(shrunk[1]);
    shrunk[0] * shrunk[0] + shrunk[2] * shrunk[2] <= reach * reach
  }

  /// The crown's centre.
  pub fn centre(&self) -> V3 {
    [self.shift, (self.bottom + self.top) * 0.5, 0.0]
  }

  fn scaled(&self, k: f32) -> Self {
    Self {
      bottom: self.bottom * k,
      top: self.top * k,
      radius: self.radius * k,
      shift: self.shift * k,
      ..*self
    }
  }
}

/// A node of the grown branch graph.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
  /// Position in metres, the trunk base at the origin.
  pub position: V3,
  /// Parent node, or `u32::MAX` for the root.
  pub parent: u32,
  /// Pipe-model radius in metres.
  pub radius: f32,
  /// Branch order: 0 the trunk, 1 a limb leaving it, and so on. At each
  /// fork the thickest child keeps its parent's order.
  pub order: u8,
  /// Nodes to the furthest twig tip beyond this one (0 at a tip).
  pub tip_depth: u16,
  /// Children, in growth order.
  pub children: Vec<u32>,
  /// The limb this node belongs to: the node on the trunk it leaves
  /// (its pivot for branch wind), or `u32::MAX` on the trunk.
  pub limb: u32,
  can_branch: bool,
}

/// A foliage clump card.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Card {
  /// Where the card hangs from its twig.
  pub attach: V3,
  /// Half extent across the card.
  pub right: V3,
  /// Half extent along the card, from `attach` outwards.
  pub up: V3,
  /// Bent normal: mostly away from the crown centre.
  pub normal: V3,
  /// Twig node the card hangs from.
  pub node: u32,
  /// Colour variation, 0 to 1.
  pub colour: f32,
  /// Whether the card is two crossed quads (needles).
  pub crossed: bool,
}

/// A grown tree: its branch graph and foliage.
#[derive(Clone, Debug, PartialEq)]
pub struct Grown {
  /// The species.
  pub species: TreeSpecies,
  /// The age class.
  pub age: Age,
  /// Branch nodes, parents before children.
  pub nodes: Vec<Node>,
  /// Foliage cards.
  pub cards: Vec<Card>,
  /// The crown envelope, at the tree's final size.
  pub envelope: Envelope,
  /// Where the knees of a swamp cypress stand: base centre, height and
  /// radius.
  pub knees: Vec<(V3, f32, f32)>,
  /// Palm fronds: base, direction, length and droop.
  pub fronds: Vec<(V3, V3, f32, f32)>,
}

const NONE: u32 = u32::MAX;
/// The pipe-model exponent.
pub const PIPE_EXPONENT: f32 = 2.5;
/// Metres the trunk's mesh extends below the origin, so a grounded tree
/// never shows a gap on a slope.
pub const ROOT_DEPTH: f32 = 0.4;
/// The root flare rises to this height, widening the trunk below it.
pub const FLARE_HEIGHT: f32 = 0.6;
/// How much wider the trunk is at the ground than above the flare.
pub const FLARE_WIDTH: f32 = 1.6;

/// How one species grows.
#[derive(Clone, Copy, Debug)]
struct Form {
  /// Height of a mature tree in metres, to which every age is scaled.
  height: f32,
  envelope: Envelope,
  /// The bare trunk is seeded to this height (share of the height).
  trunk: f32,
  /// Trunk nodes below this share of the height never branch (the lower
  /// trunk self-prunes).
  branch_from: f32,
  /// Branches leave the trunk only in whorls this far apart (metres).
  whorls: Option<(f32, f32)>,
  /// Attraction points.
  points: u32,
  /// Sub-crowns attraction points are scattered in; 0 fills the envelope.
  lobes: u32,
  /// Growth step in metres.
  step: f32,
  /// Influence and kill distances, in steps.
  influence: f32,
  kill: f32,
  /// Upward tropism.
  up: f32,
  /// Outward (and, lower down, downward) droop.
  droop: f32,
  /// Node budget.
  nodes: u32,
  /// Twig tip radius in metres.
  tip: f32,
  bark: f32,
  leaf: f32,
  /// Card size in metres.
  card: f32,
  /// Cards per twig node, at least and at most.
  cards: (u32, u32),
  /// Nodes this close to a twig tip carry cards: conifers clothe their
  /// branches in needles, broadleaves leaf at their twigs.
  leafy: u16,
  crossed: bool,
  /// Stems rising from the ground (shrubs); 0 grows one trunk.
  stems: u32,
}

const fn envelope(bottom: f32, top: f32, radius: f32, profile: Profile) -> Envelope {
  Envelope {
    bottom,
    top,
    radius,
    shift: 0.0,
    profile,
  }
}

fn form(species: TreeSpecies) -> Form {
  let base = Form {
    height: 16.0,
    envelope: envelope(0.3, 1.0, 8.0, Profile::Ellipsoid),
    trunk: 0.26,
    branch_from: 0.2,
    whorls: None,
    points: 700,
    lobes: 6,
    step: 0.5,
    influence: 4.0,
    kill: 1.6,
    up: 0.15,
    droop: 0.0,
    nodes: 560,
    tip: 0.012,
    bark: layers::BARK_OAK,
    leaf: layers::LEAF_OAK,
    card: 1.1,
    cards: (2, 3),
    leafy: 2,
    crossed: false,
    stems: 0,
  };

  match species {
    // Broad and low-forking, with heavy, crooked limbs and wide gaps.
    TreeSpecies::Oak => Form {
      card: 1.2,
      leafy: 3,
      nodes: 720,
      points: 820,
      ..base
    },
    // A tall, self-pruned trunk under an irregular, flat-topped crown.
    TreeSpecies::Pine => Form {
      height: 22.5,
      envelope: envelope(0.62, 1.0, 5.0, Profile::Ellipsoid),
      trunk: 0.8,
      branch_from: 0.6,
      points: 520,
      lobes: 5,
      up: 0.25,
      nodes: 420,
      bark: layers::BARK_PINE,
      leaf: layers::NEEDLES,
      card: 1.2,
      cards: (2, 3),
      leafy: 4,
      crossed: true,
      ..base
    },
    // A narrow cone of whorled, drooping branches.
    TreeSpecies::Spruce => Form {
      height: 20.5,
      envelope: envelope(0.06, 1.0, 4.6, Profile::Cone),
      trunk: 0.97,
      branch_from: 0.07,
      whorls: Some((0.5, 0.8)),
      points: 760,
      lobes: 0,
      step: 0.45,
      influence: 3.5,
      kill: 1.4,
      up: 0.0,
      droop: 0.45,
      nodes: 560,
      bark: layers::BARK_PINE,
      leaf: layers::SPRUCE,
      card: 1.1,
      cards: (2, 2),
      leafy: 6,
      ..base
    },
    // Palms are built, not grown (`grow_palm`).
    TreeSpecies::Palm => Form {
      height: 13.8,
      envelope: envelope(0.75, 1.0, 8.8, Profile::Umbrella),
      bark: layers::BARK_PALM,
      leaf: layers::PALM_FROND,
      ..base
    },
    // A tall, straight trunk under an umbrella crown, buttressed.
    TreeSpecies::Jungle => Form {
      height: 30.0,
      envelope: envelope(0.6, 1.0, 10.8, Profile::Umbrella),
      trunk: 0.66,
      branch_from: 0.58,
      points: 760,
      lobes: 7,
      step: 0.6,
      up: 0.12,
      nodes: 860,
      leafy: 3,
      bark: layers::BARK_SMOOTH,
      leaf: layers::LEAF_TROPICAL,
      card: 1.2,
      ..base
    },
    // A narrow column over a flared base.
    TreeSpecies::Cypress => Form {
      height: 19.8,
      envelope: envelope(0.12, 1.0, 3.4, Profile::Column),
      trunk: 0.9,
      branch_from: 0.12,
      points: 700,
      lobes: 4,
      step: 0.45,
      influence: 3.5,
      up: 0.35,
      nodes: 540,
      bark: layers::BARK_PINE,
      leaf: layers::CYPRESS,
      card: 1.1,
      cards: (3, 3),
      leafy: 5,
      ..base
    },
    // Shallow forks from a short trunk into a wide, flat top.
    TreeSpecies::Acacia => Form {
      height: 7.6,
      envelope: envelope(0.5, 1.0, 7.4, Profile::Umbrella),
      trunk: 0.3,
      branch_from: 0.25,
      points: 620,
      lobes: 5,
      step: 0.4,
      up: 0.05,
      droop: -0.25,
      nodes: 520,
      leafy: 3,
      tip: 0.01,
      leaf: layers::LEAF_FINE,
      card: 1.1,
      ..base
    },
    // Many stems from the ground in a dense dome.
    TreeSpecies::Shrub => Form {
      height: 2.2,
      envelope: envelope(0.0, 1.0, 1.6, Profile::Dome),
      trunk: 0.0,
      branch_from: 0.0,
      points: 420,
      lobes: 0,
      step: 0.16,
      influence: 4.0,
      kill: 1.5,
      up: 0.2,
      nodes: 300,
      tip: 0.006,
      leaf: layers::LEAF_BROAD,
      card: 0.55,
      cards: (2, 3),
      leafy: 3,
      stems: 5,
      ..base
    },
  }
}

/// The height of a mature tree of `species` before instance scaling, in
/// metres (the tallest vertex is within a card's reach of it).
pub fn species_height(species: TreeSpecies) -> f32 {
  form(species).height
}

/// How strongly the wind bends a species' trunk (per metre of height),
/// and how much its leaves flutter: aspen-like broadleaves most, needles
/// least. Read by `trees.wgsl` from `WorldInfo.species`.
pub fn species_wind(species: TreeSpecies) -> (f32, f32) {
  match species {
    TreeSpecies::Oak => (0.8, 1.0),
    TreeSpecies::Pine => (1.0, 0.35),
    TreeSpecies::Spruce => (0.9, 0.3),
    TreeSpecies::Palm => (1.2, 0.8),
    TreeSpecies::Jungle => (0.6, 0.9),
    TreeSpecies::Cypress => (0.8, 0.4),
    TreeSpecies::Acacia => (0.7, 0.7),
    TreeSpecies::Shrub => (0.5, 1.0),
  }
}

/// The seed of one species, variant and age class.
fn seed(species: TreeSpecies, variant: usize, age: Age) -> u64 {
  0x7ee5_0000_u64 ^ (species as u64) << 16 ^ (variant as u64) << 8 ^ age as u64
}

/// Apply an age class to a species' form.
fn aged(mut f: Form, species: TreeSpecies, age: Age) -> Form {
  match age {
    Age::Mature => {}
    Age::Young | Age::Krummholz => {
      f.envelope.radius *= 0.72;
      f.envelope.bottom = (f.envelope.bottom * 0.8).min(f.envelope.top - 0.25);
      f.points = f.points * 3 / 5;
      f.nodes = f.nodes * 11 / 20;
      f.up += 0.1;
      f.card *= 0.85;

      if species == TreeSpecies::Shrub && age == Age::Young {
        // Dwarf tundra and alpine shrubs: low, wide and dense, with
        // small leaves, spreading sideways.
        f.envelope = envelope(0.0, 0.5, 2.2, Profile::Dome);
        f.points = 420;
        f.nodes = 260;
        f.up = -0.05;
        f.droop = -0.35;
        f.card = 0.34;
        f.cards = (2, 3);
        f.stems = 7;
      }
    }
    Age::Old => {
      f.envelope.radius *= 1.15;
      f.points = f.points * 11 / 10;
      f.nodes = f.nodes * 6 / 5;
      f.kill *= 1.25;
      f.lobes += 2;
      f.up -= 0.08;
      f.droop += 0.12;
    }
  }

  if age == Age::Krummholz {
    // The crown streams leeward, away from +x.
    f.envelope.shift = -f.envelope.radius * 0.3;
    f.droop += 0.1;
  }

  f
}

/// A uniform grid over the tree's bounds, for neighbour searches.
struct Grid {
  origin: V3,
  cell: f32,
  size: [i32; 3],
  cells: Vec<Vec<u32>>,
}

impl Grid {
  fn new(low: V3, high: V3, cell: f32) -> Self {
    let size = [0, 1, 2].map(|axis| (((high[axis] - low[axis]) / cell).ceil() as i32).max(1));
    Self {
      origin: low,
      cell,
      size,
      cells: vec![Vec::new(); (size[0] * size[1] * size[2]) as usize],
    }
  }

  fn coords(&self, p: V3) -> [i32; 3] {
    [0, 1, 2].map(|axis| {
      (((p[axis] - self.origin[axis]) / self.cell).floor() as i32).clamp(0, self.size[axis] - 1)
    })
  }

  fn insert(&mut self, p: V3, item: u32) {
    let [x, y, z] = self.coords(p);
    let index = ((z * self.size[1] + y) * self.size[0] + x) as usize;
    self.cells[index].push(item);
  }

  /// Visit every item in the cells around `p`, keeping those `visit`
  /// returns true for.
  fn near(&mut self, p: V3, mut visit: impl FnMut(u32) -> bool) {
    let [x, y, z] = self.coords(p);

    for cz in (z - 1).max(0)..=(z + 1).min(self.size[2] - 1) {
      for cy in (y - 1).max(0)..=(y + 1).min(self.size[1] - 1) {
        for cx in (x - 1).max(0)..=(x + 1).min(self.size[0] - 1) {
          let index = ((cz * self.size[1] + cy) * self.size[0] + cx) as usize;

          self.cells[index].retain(|item| visit(*item));
        }
      }
    }
  }
}

/// Grow one tree.
pub fn grow(species: TreeSpecies, variant: usize, age: Age) -> Grown {
  let base = form(species);
  let f = aged(base, species, age);
  let mut rng = Rng(seed(species, variant, age));

  if species == TreeSpecies::Palm {
    return grow_palm(&f, age, &mut rng);
  }

  let h = f.height;
  let env = Envelope {
    bottom: f.envelope.bottom * h,
    top: f.envelope.top * h,
    ..f.envelope
  };
  let mut nodes: Vec<Node> = Vec::with_capacity(f.nodes as usize + 64);
  let push = |nodes: &mut Vec<Node>, position: V3, parent: u32, can_branch: bool| {
    if parent != NONE {
      let index = nodes.len() as u32;
      nodes[parent as usize].children.push(index);
    }

    nodes.push(Node {
      position,
      parent,
      radius: 0.0,
      order: 0,
      tip_depth: 0,
      children: Vec::new(),
      limb: NONE,
      can_branch,
    });
    nodes.len() as u32 - 1
  };

  // Seed the trunk (or the stems of a shrub), leaning a little.
  push(&mut nodes, [0.0, 0.0, 0.0], NONE, f.stems > 0);

  if f.stems > 0 {
    let spin = rng.unit() * std::f32::consts::TAU;

    for stem in 0..f.stems {
      let yaw = spin + stem as f32 / f.stems as f32 * std::f32::consts::TAU + rng.range(-0.3, 0.3);
      let tilt = rng.range(0.3, 0.75) + if age == Age::Young { 0.5 } else { 0.0 };
      let heading = [tilt.sin() * yaw.sin(), tilt.cos(), tilt.sin() * yaw.cos()];
      let mut parent = 0;
      let mut position = [0.0, 0.0, 0.0];

      for _ in 0..2 {
        position = add(position, scale(heading, f.step));
        parent = push(&mut nodes, position, parent, true);
      }
    }
  } else {
    let lean = [rng.range(-0.04, 0.04), 1.0, rng.range(-0.04, 0.04)];
    let top = f.trunk * h;
    let mut parent = 0;
    let mut position = [0.0, 0.0, 0.0];
    let mut whorl = f
      .whorls
      .map(|(low, high)| f.branch_from * h + rng.range(low, high));

    while position[1] + f.step <= top {
      let wobble = [rng.range(-0.03, 0.03), 0.0, rng.range(-0.03, 0.03)];
      position = add(position, scale(normalise(add(lean, wobble)), f.step));
      let mut can_branch = position[1] >= f.branch_from * h;

      if let (Some(next), Some((low, high))) = (whorl, f.whorls) {
        can_branch = can_branch && position[1] >= next;

        if can_branch {
          whorl = Some(position[1] + rng.range(low, high));
        }
      }

      parent = push(&mut nodes, position, parent, can_branch);
    }

    nodes[parent as usize].can_branch = true;
  }

  // Attraction points, in the envelope's lobes when it has any.
  let lobes: Vec<(V3, f32)> = (0..f.lobes)
    .map(|_| {
      let y = rng.range(env.bottom, env.top);
      let reach = env.radius_at(y);
      let angle = rng.unit() * std::f32::consts::TAU;
      let out = rng.range(0.25, 0.75) * reach;
      let centre = [env.shift + out * angle.sin(), y, out * angle.cos()];
      (
        centre,
        rng.range(0.45, 0.7) * env.radius.max(env.top - env.bottom),
      )
    })
    .collect();
  let wide = env.radius + env.shift.abs() + 1.0;
  let low = [-wide, -1.0, -wide];
  let high = [wide, env.top + 2.0, wide];
  let mut points = Vec::with_capacity(f.points as usize);
  let mut attempts = 0;

  while points.len() < f.points as usize && attempts < f.points * 40 {
    attempts += 1;
    let b = rng.ball();
    let span = (env.top - env.bottom) * 0.5;
    let p = [
      env.shift + b[0] * env.radius,
      (env.bottom + env.top) * 0.5 + b[1] * span,
      b[2] * env.radius,
    ];

    if !env.contains(p, 1.0) {
      continue;
    }

    // Krummholz: the windward half dies back.
    if age == Age::Krummholz && p[0] > env.radius * 0.1 {
      continue;
    }

    if !lobes.is_empty()
      && !lobes
        .iter()
        .any(|(centre, radius)| length(sub(p, *centre)) <= *radius)
    {
      continue;
    }

    points.push(p);
  }

  colonise(&mut nodes, &points, &f, &env, low, high, &push);
  let mut grown = Grown {
    species,
    age,
    nodes,
    cards: Vec::new(),
    envelope: env,
    knees: Vec::new(),
    fronds: Vec::new(),
  };
  finish(&mut grown, &f, &mut rng);

  if species == TreeSpecies::Cypress {
    // Knees rise around a swamp cypress; the shaders sink them into dry
    // ground.
    let count = 6 + (rng.unit() * 4.0) as usize;

    for _ in 0..count {
      let angle = rng.unit() * std::f32::consts::TAU;
      let reach = rng.range(1.2, 2.8);
      grown.knees.push((
        [reach * angle.sin(), 0.0, reach * angle.cos()],
        rng.range(0.25, 0.6),
        rng.range(0.07, 0.13),
      ));
    }
  }

  grown
}

/// The space colonisation loop.
#[allow(clippy::too_many_arguments)]
fn colonise(
  nodes: &mut Vec<Node>,
  points: &[V3],
  f: &Form,
  env: &Envelope,
  low: V3,
  high: V3,
  push: &impl Fn(&mut Vec<Node>, V3, u32, bool) -> u32,
) {
  let influence = f.influence * f.step;
  let kill = (f.kill * f.step).powi(2);
  let mut point_grid = Grid::new(low, high, influence);
  let mut alive = vec![true; points.len()];
  // Each point's nearest node that may branch, and how far it is.
  let mut nearest = vec![(NONE, f32::MAX); points.len()];

  for (index, p) in points.iter().enumerate() {
    point_grid.insert(*p, index as u32);
  }

  let mut fresh: Vec<u32> = Vec::new();

  for (index, node) in nodes.iter().enumerate() {
    if node.can_branch {
      fresh.push(index as u32);
    }
  }

  let mut pull: Vec<V3> = Vec::new();
  let mut pulled: Vec<usize> = Vec::new();
  let mut live: Vec<u32> = (0..points.len() as u32).collect();
  let centre = env.centre();

  for _ in 0..400 {
    // New nodes kill the points they reach and become the nearest node of
    // the points around them.
    for node in &fresh {
      let position = nodes[*node as usize].position;
      point_grid.near(position, |point| {
        let point = point as usize;
        let d = sub(points[point], position);
        let distance = dot(d, d);

        if distance < kill {
          alive[point] = false;
          return false;
        }

        if distance < nearest[point].1 {
          nearest[point] = (*node, distance);
        }

        true
      });
    }

    // Only the points still alive pull, and only the nodes they pull
    // can grow.
    live.retain(|point| alive[*point as usize]);
    pull.resize(nodes.len(), [0.0; 3]);

    for point in &live {
      let (node, distance) = nearest[*point as usize];

      if node != NONE && distance < influence * influence {
        let node = node as usize;

        if pull[node] == [0.0; 3] {
          pulled.push(node);
        }

        let towards = normalise(sub(points[*point as usize], nodes[node].position));
        pull[node] = add(pull[node], towards);
      }
    }

    pulled.sort_unstable();
    fresh.clear();
    let reached = !pulled.is_empty();

    for index in pulled.drain(..) {
      let sum = pull[index];
      pull[index] = [0.0; 3];

      if nodes.len() >= f.nodes as usize || length(sum) < 1e-3 {
        continue;
      }

      let position = nodes[index].position;
      let height = ((position[1] - env.bottom) / (env.top - env.bottom).max(1e-3)).clamp(0.0, 1.0);
      let outward = normalise([position[0] - centre[0], 0.0, position[2] - centre[2]]);
      // Droop grows lower in the crown; a negative droop spreads branches
      // sideways and up.
      let tropism = add(
        [0.0, f.up, 0.0],
        add(
          scale(outward, f.droop.abs() * 0.6),
          [0.0, -f.droop * (1.0 - height), 0.0],
        ),
      );
      let heading = normalise(add(normalise(sum), tropism));
      let repeated = nodes[index].children.iter().any(|child| {
        let child = &nodes[*child as usize];
        dot(normalise(sub(child.position, position)), heading) > 0.9
      });

      if repeated {
        continue;
      }

      let next = add(position, scale(heading, f.step));
      let node = push(nodes, next, index as u32, true);
      fresh.push(node);
    }

    if fresh.is_empty() {
      // Nothing reaches the crown yet: the leader grows on up towards
      // it.
      let leader = (0..nodes.len())
        .max_by(|a, b| nodes[*a].position[1].total_cmp(&nodes[*b].position[1]))
        .unwrap_or(0);
      let position = nodes[leader].position;
      if reached || position[1] + f.step > env.top || nodes.len() >= f.nodes as usize {
        break;
      }

      let next = add(position, [0.0, f.step, 0.0]);
      let node = push(nodes, next, leader as u32, true);
      fresh.push(node);
    }
  }
}

/// Radii, orders, limbs and foliage for a grown branch graph, then scale
/// it to the species' height.
fn finish(grown: &mut Grown, f: &Form, rng: &mut Rng) {
  let nodes = &mut grown.nodes;

  // Scale to the species' height (a dwarf shrub keeps its own).
  let top = nodes
    .iter()
    .map(|node| node.position[1])
    .fold(0.1, f32::max);
  let target = if grown.species == TreeSpecies::Shrub && grown.age == Age::Young {
    f.height * 0.3
  } else {
    f.height
  } - f.card * 0.45;
  let k = target / top;

  for node in nodes.iter_mut() {
    node.position = scale(node.position, k);
  }

  grown.envelope = grown.envelope.scaled(k);
  pipe_radii(nodes, grown.species, f.tip);

  // Branch orders: at each fork the thickest child continues its
  // parent's order.
  for index in 0..nodes.len() {
    let order = nodes[index].order;
    let children = nodes[index].children.clone();
    let thickest = children.iter().copied().max_by(|a, b| {
      nodes[*a as usize]
        .radius
        .total_cmp(&nodes[*b as usize].radius)
    });

    for child in children {
      let continues = Some(child) == thickest && !(index == 0 && f.stems > 0);
      nodes[child as usize].order = if continues { order } else { order + 1 };
      nodes[child as usize].limb = if nodes[child as usize].order == 0 {
        NONE
      } else if order == 0 {
        index as u32
      } else {
        nodes[index].limb
      };
    }
  }

  for index in (0..nodes.len()).rev() {
    let depth = nodes[index]
      .children
      .iter()
      .map(|child| nodes[*child as usize].tip_depth + 1)
      .max()
      .unwrap_or(0);
    nodes[index].tip_depth = depth;
  }

  // Clump cards at twig tips and the node behind each.
  let centre = grown.envelope.centre();
  let radii = [
    grown.envelope.radius.max(0.5),
    ((grown.envelope.top - grown.envelope.bottom) * 0.5).max(0.5),
    grown.envelope.radius.max(0.5),
  ];

  for (index, node) in nodes.iter().enumerate() {
    if node.tip_depth > f.leafy || (node.order == 0 && !node.children.is_empty()) {
      continue;
    }

    let count = f.cards.0 + (rng.unit() * (f.cards.1 - f.cards.0 + 1) as f32) as u32;
    let from = if node.parent == NONE {
      node.position
    } else {
      nodes[node.parent as usize].position
    };
    let twig = normalise(sub(node.position, from));

    for _ in 0..count.min(f.cards.1) {
      let offset = sub(node.position, centre);
      // The gradient of the ellipsoidal crown: its outward normal.
      let outward = normalise([
        offset[0] / (radii[0] * radii[0]),
        offset[1] / (radii[1] * radii[1]),
        offset[2] / (radii[2] * radii[2]),
      ]);
      let jitter = scale(rng.ball(), 0.6);
      let facing = normalise(add(add(outward, jitter), scale(twig, 0.3)));
      let size = f.card * rng.range(0.8, 1.15);
      // Along the card: out along the twig, rolled at random about the
      // card's normal.
      let along = rotate(
        normalise(sub(twig, scale(facing, dot(twig, facing)))),
        facing,
        rng.range(-0.9, 0.9),
      );
      let across = normalise(cross(facing, along));
      let bent = normalise(add(scale(outward, 0.65), scale(facing, 0.35)));
      grown.cards.push(Card {
        attach: add(node.position, scale(scale(rng.ball(), 0.3), f.card)),
        right: scale(across, size * 0.5),
        up: scale(along, size * 0.5),
        normal: bent,
        node: index as u32,
        colour: rng.unit(),
        crossed: f.crossed,
      });
    }
  }
}

/// Pipe-model radii: tips take `tip`, each node along a branch adds the
/// pipes of the foliage it carries, and forks sum their children exactly.
/// The whole is then scaled so the trunk at breast height matches the
/// species' trunk radius.
fn pipe_radii(nodes: &mut [Node], species: TreeSpecies, tip: f32) {
  // Pipes in tip units, twigs first.
  let mut pipes = vec![0.0f32; nodes.len()];
  let mut twigs = vec![0.0f32; nodes.len()];
  let mut along = vec![0.0f32; nodes.len()];

  for index in (0..nodes.len()).rev() {
    let children = &nodes[index].children;

    match children.len() {
      0 => twigs[index] = 1.0,
      1 => {
        let child = children[0] as usize;
        twigs[index] = twigs[child];
        along[index] = along[child] + 1.0;
      }
      _ => {
        for child in children {
          twigs[index] += twigs[*child as usize];
          along[index] += along[*child as usize];
        }
      }
    }
  }

  // Foliage pipes per branch node, so the trunk at breast height carries
  // the species' radius with twigs of the tip radius.
  let breast = breast_node(nodes);
  let wanted = (core_radius(species) / tip).powf(PIPE_EXPONENT);
  let foliage = if species == TreeSpecies::Shrub {
    2.0
  } else if along[breast] > 0.0 {
    ((wanted - twigs[breast]) / along[breast]).max(0.0)
  } else {
    0.0
  };

  for index in (0..nodes.len()).rev() {
    let children = &nodes[index].children;
    pipes[index] = match children.len() {
      0 => 1.0,
      1 => pipes[children[0] as usize] + foliage,
      _ => children.iter().map(|child| pipes[*child as usize]).sum(),
    };
  }

  let unit = if species == TreeSpecies::Shrub {
    tip
  } else {
    core_radius(species) / pipes[breast].powf(1.0 / PIPE_EXPONENT)
  };

  for (node, pipe) in nodes.iter_mut().zip(pipes) {
    node.radius = unit * pipe.powf(1.0 / PIPE_EXPONENT);
  }
}

/// The trunk's radius at breast height without buttresses: a rainforest
/// emergent's fins reach out to its root radius.
fn core_radius(species: TreeSpecies) -> f32 {
  species_trunk_radius(species)
    * if species == TreeSpecies::Jungle {
      0.8
    } else {
      1.0
    }
}

/// The trunk node nearest breast height.
fn breast_node(nodes: &[Node]) -> usize {
  let mut index = 0;

  while let Some(next) = nodes[index]
    .children
    .iter()
    .find(|child| nodes[**child as usize].position[1] <= TRUNK_BREAST_HEIGHT + 0.3)
  {
    index = *next as usize;
  }

  index
}

/// Palms are not colonised: a curved, ringed trunk and a crown of 12 to
/// 18 fronds.
fn grow_palm(f: &Form, age: Age, rng: &mut Rng) -> Grown {
  let h = f.height;
  let segments = 18;
  let lean = rng.unit() * std::f32::consts::TAU;
  let (sin, cos) = lean.sin_cos();
  let bend = match age {
    Age::Young => 0.05,
    Age::Old | Age::Krummholz => 0.22,
    Age::Mature => 0.14,
  };
  let trunk_top = h * 0.88;
  let mut nodes = Vec::new();

  // A gentle S-curve: lean out, then curve back up towards the light.
  for i in 0..=segments {
    let t = i as f32 / segments as f32;
    let out = bend * h * (t * std::f32::consts::PI * 0.8).sin() * t;
    nodes.push(Node {
      position: [sin * out, t * trunk_top, cos * out],
      parent: if i == 0 { NONE } else { i - 1 },
      radius: 0.0,
      order: 0,
      tip_depth: (segments - i) as u16,
      children: if i < segments {
        vec![i + 1]
      } else {
        Vec::new()
      },
      limb: NONE,
      can_branch: false,
    });
  }

  // Palm trunks barely taper; their radius is set by the species table.
  let radius = species_trunk_radius(TreeSpecies::Palm);

  for (i, node) in nodes.iter_mut().enumerate() {
    let t = i as f32 / segments as f32;
    node.radius = radius * (1.0 - 0.25 * t);
  }

  let top = nodes[segments as usize].position;
  let count = 12 + (rng.unit() * 7.0) as usize;
  let mut fronds = Vec::with_capacity(count);

  for frond in 0..count.min(18) {
    let young = frond >= count - 3;
    let yaw = frond as f32 / count as f32 * std::f32::consts::TAU * 1.618 + rng.range(-0.2, 0.2);
    let elevation = if young {
      rng.range(0.9, 1.25)
    } else {
      rng.range(0.1, 0.6)
    };
    let length = if young {
      rng.range(2.5, 3.2)
    } else {
      rng.range(4.6, 5.8)
    } * if age == Age::Young { 0.8 } else { 1.0 };
    let heading = [
      elevation.cos() * yaw.sin(),
      elevation.sin(),
      elevation.cos() * yaw.cos(),
    ];
    fronds.push((top, heading, length, if young { 0.3 } else { 1.9 }));
  }

  Grown {
    species: TreeSpecies::Palm,
    age,
    nodes,
    cards: Vec::new(),
    envelope: Envelope {
      bottom: f.envelope.bottom * h,
      top: f.envelope.top * h,
      ..f.envelope
    },
    knees: Vec::new(),
    fronds,
  }
}

/// Mesh building, with the wind data every vertex carries.
struct Builder {
  vertices: Vec<TreeVertex>,
  indices: Vec<u32>,
  /// Crown centre and half extents, for occlusion.
  centre: V3,
  extent: V3,
}

/// Wind data of one primitive.
#[derive(Clone, Copy)]
struct Wind {
  pivot: V3,
  /// 0 trunk, 1 limb, 2 twig, 3 leaf.
  level: f32,
  stiffness: f32,
  phase: f32,
}

impl Builder {
  #[allow(clippy::too_many_arguments)]
  fn push(
    &mut self,
    position: V3,
    normal: V3,
    uv: [f32; 2],
    layer: f32,
    ao: f32,
    wind: Wind,
    extra: f32,
  ) -> u32 {
    let index = self.vertices.len() as u32;
    self.vertices.push(TreeVertex {
      position,
      normal: pack_normal(normalise(normal), wind.level * 0.25 + extra * 0.2),
      uv,
      params: [layer, wind.stiffness, ao, wind.phase],
      pivot: wind.pivot,
    });
    index
  }

  /// How deep inside the crown a point is, as occlusion: 1 at its
  /// surface and in the open.
  fn occlusion(&self, p: V3) -> f32 {
    let d = sub(p, self.centre);
    let reach = (0..3)
      .map(|axis| (d[axis] / self.extent[axis].max(0.5)).powi(2))
      .sum::<f32>()
      .sqrt();
    0.5 + 0.5 * reach.clamp(0.0, 1.0)
  }

  /// A generalised cylinder through `points` with a radius and wind per
  /// point, and `ring(i, angle)` scaling each ring's radius around it.
  /// Parallel-transport frames keep it from twisting, and bark `v` runs
  /// with length at `u` repeats around, so texels stay square.
  #[allow(clippy::too_many_arguments)]
  fn tube(
    &mut self,
    points: &[V3],
    radii: &[f32],
    winds: &[Wind],
    sides: u32,
    layer: f32,
    extra: f32,
    ring: Option<&dyn Fn(usize, f32) -> f32>,
  ) {
    if points.len() < 2 || sides < 3 {
      return;
    }

    let mut tangent = normalise(sub(points[1], points[0]));
    let mut normal = perpendicular(tangent);
    // Whole repeats around, and texels as long as they are wide.
    let circumference = std::f32::consts::TAU * radii[0].max(0.01);
    let repeats = (circumference / 0.9).round().max(1.0);
    let tile = circumference / repeats;
    let first = self.vertices.len() as u32;
    let mut v = 0.0;
    let trig: Vec<(f32, f32, f32)> = (0..=sides)
      .map(|side| {
        let angle = side as f32 / sides as f32 * std::f32::consts::TAU;
        let (s, c) = angle.sin_cos();
        (angle, s, c)
      })
      .collect();

    for (i, point) in points.iter().enumerate() {
      let next = if i + 1 < points.len() {
        normalise(sub(points[i + 1], *point))
      } else {
        tangent
      };
      let blended = normalise(add(tangent, next));
      normal = normalise(sub(normal, scale(blended, dot(normal, blended))));
      let binormal = cross(blended, normal);

      if i > 0 {
        v += length(sub(*point, points[i - 1])) / tile;
      }

      let ao = self.occlusion(*point).min(if point[1] < 1.0 {
        0.55 + 0.45 * point[1].max(0.0)
      } else {
        1.0
      });

      for (side, (angle, s, c)) in trig.iter().enumerate() {
        let radial = add(scale(normal, *c), scale(binormal, *s));
        let (r, shading) = match ring {
          None => (radii[i], radial),
          Some(ring) => {
            // Where the radius varies around the ring (buttresses), tilt
            // the normal against its slope.
            let step = 0.05;
            let r = radii[i] * ring(i, *angle);
            let slope = (ring(i, angle + step) - ring(i, angle - step)) / (2.0 * step) * radii[i];
            let tangential = add(scale(normal, -s), scale(binormal, *c));
            (r, sub(scale(radial, r.max(1e-4)), scale(tangential, slope)))
          }
        };
        self.push(
          add(*point, scale(radial, r)),
          shading,
          [side as f32 / sides as f32 * repeats, v],
          layer,
          ao,
          winds[i],
          extra,
        );
      }

      tangent = next;
    }

    let stride = sides + 1;

    for i in 0..(points.len() as u32 - 1) {
      for side in 0..sides {
        let a = first + i * stride + side;
        let b = a + 1;
        let c = a + stride;
        let d = c + 1;
        self.indices.extend_from_slice(&[a, c, b, b, c, d]);
      }
    }
  }

  /// A leaf card, or two crossed quads, hanging from `card.attach`.
  fn card(&mut self, card: &Card, grow: f32, layer: f32, wind: Wind) {
    let planes: &[V3] = if card.crossed {
      &[card.right, cross(normalise(card.up), card.right)]
    } else {
      &[card.right]
    };
    let up = scale(card.up, grow);
    let ao = self.occlusion(add(card.attach, up));

    for right in planes {
      let right = scale(*right, grow);
      let first = self.vertices.len() as u32;
      let flip = card.colour > 0.5;

      for (along, across) in [(0.0, -1.0), (0.0, 1.0), (2.0, 1.0), (2.0, -1.0)] {
        let position = add(add(card.attach, scale(up, along)), scale(right, across));
        let u = if flip {
          0.5 - across * 0.5
        } else {
          0.5 + across * 0.5
        };
        self.push(
          position,
          card.normal,
          [u, along * 0.5],
          layer,
          ao,
          wind,
          card.colour,
        );
      }

      self
        .indices
        .extend_from_slice(&[first, first + 1, first + 2, first, first + 2, first + 3]);
    }
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

/// The phase of a limb's bob, from its pivot node.
fn limb_phase(limb: u32) -> f32 {
  (limb as f32 * 0.618_034).fract() * std::f32::consts::TAU
}

/// The root flare: how much wider the trunk is at `height` metres.
pub fn flare(height: f32) -> f32 {
  let t = (height / FLARE_HEIGHT).clamp(0.0, 1.0);
  1.0 + (FLARE_WIDTH - 1.0) * (1.0 - t * t * (3.0 - 2.0 * t))
}

/// The wind data of every node.
fn node_winds(nodes: &[Node]) -> Vec<Wind> {
  // Each limb's radius where it leaves the trunk.
  let mut limb_radius = vec![0.0f32; nodes.len()];

  for node in nodes {
    if node.order == 1 && node.limb != NONE && node.parent == node.limb {
      let slot = &mut limb_radius[node.limb as usize];
      *slot = slot.max(node.radius);
    }
  }

  nodes
    .iter()
    .map(|node| {
      if node.order == 0 || node.limb == NONE {
        return Wind {
          pivot: [0.0; 3],
          level: 0.0,
          stiffness: 1.0,
          phase: 0.0,
        };
      }

      // The limb is stiff where it leaves the trunk and free at its
      // twigs.
      let base = limb_radius[node.limb as usize].max(node.radius).max(1e-4);
      Wind {
        pivot: nodes[node.limb as usize].position,
        level: if node.order == 1 { 1.0 } else { 2.0 },
        stiffness: (node.radius / base).sqrt(),
        phase: limb_phase(node.limb),
      }
    })
    .collect()
}

/// Mesh a grown tree at a level of detail: 0 the full mesh; 1 drops
/// branches of order 3 and above, halves the tubes' sides and draws half
/// the leaf cards at 1.4 times their size.
pub fn mesh(grown: &Grown, lod: usize) -> TreeMesh {
  let f = form(grown.species);
  let env = &grown.envelope;
  let mut b = Builder {
    vertices: Vec::new(),
    indices: Vec::new(),
    centre: env.centre(),
    extent: [env.radius, (env.top - env.bottom) * 0.5, env.radius],
  };

  if grown.species == TreeSpecies::Palm {
    mesh_palm(grown, &mut b, lod);
    return b.finish();
  }

  let nodes = &grown.nodes;
  let winds = node_winds(nodes);
  let buttressed = grown.species == TreeSpecies::Jungle;
  let fins = 5;
  let fin_phase = (nodes.len() as f32 * 0.37).fract() * std::f32::consts::TAU;

  // Each chain starts at a node whose parent it leaves (or the root) and
  // follows the thickest child.
  for (start, node) in nodes.iter().enumerate() {
    let starts = node.parent == NONE || node.order != nodes[node.parent as usize].order;

    if !starts || (lod == 1 && node.order >= 3) {
      continue;
    }

    // Short twigs under the leaf clumps are hidden by them; LOD1 also
    // drops the thin branches behind them.
    if node.parent != NONE && node.tip_depth < if lod == 0 { 3 } else { 7 } {
      continue;
    }

    let mut chain = Vec::new();
    // The chain begins inside its parent, so joints close.
    if node.parent != NONE {
      chain.push(node.parent as usize);
    }

    let mut current = start;

    loop {
      chain.push(current);
      let next = nodes[current]
        .children
        .iter()
        .copied()
        .find(|child| nodes[*child as usize].order == node.order);

      match next {
        Some(child) => current = child as usize,
        None => break,
      }
    }

    // Rings where the branch bends or has run on a while, so straight,
    // thick limbs need few; LOD1 needs half as many.
    let spacing = if lod == 0 { 1.0 } else { 3.5 };
    let mut kept = vec![false; chain.len()];
    let mut last = 0;

    for position in 0..chain.len() {
      let n = &nodes[chain[position]];
      let from = nodes[chain[last]].position;
      let run = length(sub(n.position, from));
      let turn = if position + 1 < chain.len() && position > last {
        let before = normalise(sub(n.position, from));
        let after = normalise(sub(nodes[chain[position + 1]].position, n.position));
        dot(before, after)
      } else {
        1.0
      };
      let reach = spacing * (0.8 + n.radius * 4.0).min(2.5);

      if position == 0 || position + 1 == chain.len() || run >= reach || turn < 0.96 {
        kept[position] = true;
        last = position;
      }
    }

    let keep = |position: usize, _: f32| kept[position];
    let mut points = Vec::new();
    let mut radii = Vec::new();
    let mut wind = Vec::new();

    if node.parent == NONE && f.stems == 0 {
      // The root flare, reaching below the origin.
      for y in [-ROOT_DEPTH, 0.0, 0.15, 0.3, 0.45] {
        points.push([0.0, y, 0.0]);
        radii.push(nodes[0].radius);
        wind.push(winds[0]);
      }
    }

    for (position, index) in chain.iter().enumerate() {
      let n = &nodes[*index];

      if (node.parent == NONE && f.stems == 0 && n.position[1] < FLARE_HEIGHT)
        || !keep(position, n.radius)
      {
        continue;
      }

      points.push(n.position);
      // A child's first point sits inside its parent at the child's
      // radius.
      radii.push(if position == 0 && node.parent != NONE {
        node.radius
      } else {
        n.radius
      });
      wind.push(winds[*index]);
    }

    let thickest = radii.iter().copied().fold(0.0, f32::max);
    let mut sides = if thickest > 0.25 {
      10
    } else if thickest > 0.08 {
      8
    } else {
      6
    };

    if buttressed && node.order == 0 {
      sides = 20;
    }

    // Buttresses keep their sides at every level of detail.
    if lod == 1 && !(buttressed && node.order == 0) {
      sides = (sides / 2).max(3);
    }

    let trunk = node.order == 0 && node.parent == NONE;
    let ys: Vec<f32> = points.iter().map(|p| p[1]).collect();
    let ring = |i: usize, angle: f32| {
      if !trunk {
        return 1.0;
      }

      let y = ys[i];
      let mut r = flare(y);

      if buttressed && y < 3.5 {
        let fin = (angle * fins as f32 + fin_phase).cos().max(0.0).powi(8);
        r += fin * 1.6 * (1.0 - y.max(0.0) / 3.5).powf(1.6);
      }

      r
    };
    if buttressed && trunk {
      // Fins need many sides; the bole above them does not.
      let split = ys
        .iter()
        .position(|y| *y >= 3.5)
        .unwrap_or(ys.len() - 1)
        .max(1);
      let upper = if lod == 0 { 10 } else { 5 };
      b.tube(
        &points[..=split],
        &radii[..=split],
        &wind[..=split],
        sides,
        f.bark,
        0.0,
        Some(&ring),
      );
      let ring_above = |i: usize, angle: f32| ring(i + split, angle);
      b.tube(
        &points[split..],
        &radii[split..],
        &wind[split..],
        upper,
        f.bark,
        0.0,
        Some(&ring_above),
      );
    } else {
      b.tube(
        &points,
        &radii,
        &wind,
        sides,
        f.bark,
        0.0,
        trunk.then_some(&ring as &dyn Fn(usize, f32) -> f32),
      );
    }
  }

  for (base, height, radius) in &grown.knees {
    let points = [
      add(*base, [0.0, -ROOT_DEPTH, 0.0]),
      *base,
      add(*base, [0.0, height * 0.6, 0.0]),
      add(*base, [0.0, *height, 0.0]),
    ];
    let radii = [radius * 1.4, radius * 1.2, radius * 0.8, radius * 0.25];
    let still = Wind {
      pivot: [0.0; 3],
      level: 0.0,
      stiffness: 1.0,
      phase: 0.0,
    };
    // Flagged in the colour field, so dry ground sinks them.
    b.tube(
      &points,
      &radii,
      &[still; 4],
      if lod == 0 { 6 } else { 4 },
      f.bark,
      1.0,
      None,
    );
  }

  let (grow, every) = if lod == 0 { (1.0, 1) } else { (1.4, 2) };

  for (index, card) in grown.cards.iter().enumerate() {
    if index % every != 0 {
      continue;
    }

    let node = card.node as usize;
    let mut wind = winds[node];
    wind.level = 3.0;
    wind.stiffness *= 0.5;
    // At LOD1 a needle clump is one quad, not two crossed.
    let card = Card {
      crossed: card.crossed && lod == 0,
      ..*card
    };
    b.card(&card, grow, f.leaf, wind);
  }

  b.finish()
}

fn mesh_palm(grown: &Grown, b: &mut Builder, lod: usize) {
  let f = form(TreeSpecies::Palm);
  let nodes = &grown.nodes;
  let still = Wind {
    pivot: [0.0; 3],
    level: 0.0,
    stiffness: 1.0,
    phase: 0.0,
  };
  let mut points = vec![[0.0, -ROOT_DEPTH, 0.0], [0.0, 0.0, 0.0], [0.0, 0.3, 0.0]];
  let mut radii = vec![nodes[0].radius; 3];
  let step = if lod == 0 { 1 } else { 2 };

  for node in nodes.iter().skip(1).step_by(step) {
    points.push(node.position);
    radii.push(node.radius);
  }

  if points.last() != Some(&nodes[nodes.len() - 1].position) {
    points.push(nodes[nodes.len() - 1].position);
    radii.push(nodes[nodes.len() - 1].radius);
  }

  let ys: Vec<f32> = points.iter().map(|p| p[1]).collect();
  let winds = vec![still; points.len()];
  b.tube(
    &points,
    &radii,
    &winds,
    if lod == 0 { 8 } else { 4 },
    f.bark,
    0.0,
    Some(&|i, _| flare(ys[i])),
  );

  let top = nodes[nodes.len() - 1].position;
  let crown = add(top, [0.0, -0.5, 0.0]);
  let segments = if lod == 0 { 8 } else { 4 };

  for (frond, (base, heading, length, droop)) in grown.fronds.iter().enumerate() {
    let side = normalise(cross([0.0, 1.0, 0.0], *heading));
    let phase = limb_phase(frond as u32 + 7);
    let mut previous: Option<[u32; 3]> = None;

    for s in 0..=segments {
      let t = s as f32 / segments as f32;
      // The rib arches down more and more towards the tip.
      let rib = add(
        add(*base, scale(*heading, length * t)),
        [0.0, -droop * t * t * length * 0.35, 0.0],
      );
      let width = if t < 0.12 {
        t / 0.12 * 0.9
      } else {
        1.0 - (t - 0.12) * 0.75
      } * 0.8;
      // Leaflets fold up from the rib in a shallow V.
      let fold = [0.0, width * 0.35, 0.0];
      let normal = normalise(add(normalise(sub(rib, crown)), [0.0, 1.2, 0.0]));
      let wind = Wind {
        pivot: *base,
        level: 3.0,
        stiffness: (1.0 - t) * 0.6,
        phase,
      };
      let ao = 0.55 + 0.45 * t;
      let left = b.push(
        add(add(rib, scale(side, -width)), fold),
        normal,
        [0.0, t],
        f.leaf,
        ao,
        wind,
        t,
      );
      let middle = b.push(rib, normal, [0.5, t], f.leaf, ao, wind, t);
      let right = b.push(
        add(add(rib, scale(side, width)), fold),
        normal,
        [1.0, t],
        f.leaf,
        ao,
        wind,
        t,
      );

      if let Some([pl, pm, pr]) = previous {
        b.indices.extend_from_slice(&[
          pl, left, pm, pm, left, middle, pm, middle, pr, pr, middle, right,
        ]);
      }

      previous = Some([left, middle, right]);
    }
  }
}

/// Grow one species' variant and age class and mesh both levels of
/// detail.
pub fn grow_meshes(species: TreeSpecies, variant: usize, age: Age) -> [TreeMesh; LODS] {
  let grown = grow(species, variant, age);
  [mesh(&grown, 0), mesh(&grown, 1)]
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::render::tree_models::unpack_normal;

  fn every() -> impl Iterator<Item = (TreeSpecies, usize, Age)> {
    TreeSpecies::ALL.into_iter().flat_map(|species| {
      (0..VARIANTS).flat_map(move |variant| Age::ALL.map(|age| (species, variant, age)))
    })
  }

  #[test]
  fn growth_is_deterministic() {
    for species in TreeSpecies::ALL {
      let a = grow_meshes(species, 2, Age::Mature);
      let b = grow_meshes(species, 2, Age::Mature);
      assert_eq!(a, b, "{species:?}");
      // A different variant is a different tree.
      assert_ne!(a[0], grow_meshes(species, 1, Age::Mature)[0], "{species:?}");
    }
  }

  #[test]
  fn forks_follow_the_pipe_model() {
    for (species, variant, age) in every() {
      let grown = grow(species, variant, age);
      let mut forks = 0;

      for node in &grown.nodes {
        if node.children.len() < 2 {
          continue;
        }

        forks += 1;
        let parent = node.radius.powf(PIPE_EXPONENT);
        let children: f32 = node
          .children
          .iter()
          .map(|child| grown.nodes[*child as usize].radius.powf(PIPE_EXPONENT))
          .sum();
        assert!(
          (parent - children).abs() <= parent * 0.01,
          "{species:?} {variant} {age:?}: {parent} against {children}"
        );
      }

      if species != TreeSpecies::Palm {
        assert!(
          forks > 10,
          "{species:?} {variant} {age:?} forks only {forks} times"
        );
      }
    }
  }

  #[test]
  fn trunks_match_the_species_table() {
    for species in TreeSpecies::ALL {
      if species == TreeSpecies::Shrub {
        continue;
      }

      let grown = grow(species, 0, Age::Mature);
      let radius = grown.nodes[breast_node(&grown.nodes)].radius;
      let table = core_radius(species);
      assert!(
        (radius - table).abs() <= table * 0.3,
        "{species:?}: {radius} against {table}"
      );
    }
  }

  #[test]
  fn leaf_cards_stay_in_their_envelope() {
    for (species, variant, age) in every() {
      let grown = grow(species, variant, age);

      if grown.cards.is_empty() {
        assert_eq!(species, TreeSpecies::Palm);
        continue;
      }

      let inside = grown
        .cards
        .iter()
        .filter(|card| grown.envelope.contains(card.attach, 1.1))
        .count();
      assert!(
        inside as f32 >= grown.cards.len() as f32 * 0.9,
        "{species:?} {variant} {age:?}: {inside} of {}",
        grown.cards.len()
      );
    }
  }

  #[test]
  fn the_root_flare_reaches_below_the_ground() {
    for species in TreeSpecies::ALL {
      if species == TreeSpecies::Shrub {
        continue;
      }

      let mesh = mesh(&grow(species, 0, Age::Mature), 0);
      let lowest = mesh
        .vertices
        .iter()
        .map(|vertex| vertex.position[1])
        .fold(f32::MAX, f32::min);
      assert!(lowest <= -0.3, "{species:?}: {lowest}");
      // The trunk's bark within a height band, ignoring buttress fins.
      let width = |low: f32, high: f32| {
        let mut widths: Vec<f32> = mesh
          .vertices
          .iter()
          .filter(|v| {
            v.params[0] < layers::FIRST_FOLIAGE
              && (low..=high).contains(&v.position[1])
              && unpack_normal(v.normal).1 < 0.1
          })
          .map(|v| (v.position[0].powi(2) + v.position[2].powi(2)).sqrt())
          .collect();
        widths.sort_by(f32::total_cmp);
        widths.get(widths.len() / 2).copied().unwrap_or(0.0)
      };
      let base = width(-0.45, 0.01);
      let above = width(0.9, 1.1);
      assert!(
        base >= above * 1.4,
        "{species:?}: {base} at the base, {above} at 1 m"
      );
    }
  }

  #[test]
  fn dwarf_shrubs_are_low_and_wide() {
    let dwarf = mesh(&grow(TreeSpecies::Shrub, 0, Age::Young), 0);
    assert!((0.3..=0.8).contains(&dwarf.height), "{}", dwarf.height);
    assert!(
      dwarf.radius > dwarf.height,
      "{} by {}",
      dwarf.radius,
      dwarf.height
    );
  }

  #[test]
  fn krummholz_crowns_stream_leeward() {
    let grown = grow(TreeSpecies::Spruce, 0, Age::Krummholz);
    let windward = grown
      .cards
      .iter()
      .filter(|card| card.attach[0] > 0.5)
      .count();
    let leeward = grown
      .cards
      .iter()
      .filter(|card| card.attach[0] < -0.5)
      .count();
    assert!(
      leeward > windward * 3,
      "{leeward} leeward, {windward} windward"
    );
  }

  #[test]
  fn species_keep_their_proportions() {
    let mature = |species| mesh(&grow(species, 0, Age::Mature), 0);
    let (palm, spruce, acacia, shrub, jungle) = (
      mature(TreeSpecies::Palm),
      mature(TreeSpecies::Spruce),
      mature(TreeSpecies::Acacia),
      mature(TreeSpecies::Shrub),
      mature(TreeSpecies::Jungle),
    );
    assert!(spruce.height / spruce.radius > 3.0);
    assert!(acacia.radius / acacia.height > 0.6);
    assert!(shrub.height < 3.0);
    assert!(jungle.height > palm.height);
  }

  /// Growth time per species (`cargo test --release -- --ignored
  /// growth_time --nocapture`). The browser logs the real figure as the
  /// `tree growth` start-up phase.
  #[test]
  #[ignore]
  fn growth_time() {
    let mut total = std::time::Duration::ZERO;

    for species in TreeSpecies::ALL {
      let started = std::time::Instant::now();

      for variant in 0..VARIANTS {
        for age in Age::ALL {
          std::hint::black_box(grow_meshes(species, variant, age));
        }
      }

      let elapsed = started.elapsed();
      total += elapsed;
      println!("{species:?}: {:.1} ms", elapsed.as_secs_f64() * 1000.0);
    }

    println!("total: {:.1} ms", total.as_secs_f64() * 1000.0);
  }
}
