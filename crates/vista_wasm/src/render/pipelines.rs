/// Names of shader modules compiled by the renderer.
///
/// Render shaders are compiled with `common.wgsl` prepended; compute
/// shaders are standalone.
pub const SHADER_MODULES: &[&str] = &[
  "common.wgsl",
  "hydraulic_erosion.wgsl",
  "thermal_erosion.wgsl",
  "normals.wgsl",
  "texture_gen.wgsl",
  "mipgen.wgsl",
  "clipmap_render.wgsl",
  "trees.wgsl",
  "tree_cull.wgsl",
  "ground.wgsl",
  "grounding.wgsl",
  "lattice.wgsl",
  "generate_common.wgsl",
  "tree_generate.wgsl",
  "grass_generate.wgsl",
  "grass_instances.wgsl",
  "materials.wgsl",
  "boulder_generate.wgsl",
  "boulders.wgsl",
  "water.wgsl",
  "atmosphere.wgsl",
];

/// A pipeline the renderer creates only when the scene needs it. Kinds are
/// listed in the order a frame uses them, so the first frame's pipelines
/// are created, and compiled by the browser, in the order it draws.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelineKind {
  /// Terrain sun-shadow bake (compute).
  TerrainShadow,
  /// Wet ground, puddles and snow depth (compute).
  SurfaceWeather,
  /// Standing trees and grass on the drawn terrain (compute).
  Grounding,
  /// Filling streamed tree tiles near the camera (compute).
  TreeGenerate,
  /// Tree culling and level-of-detail sorting (compute).
  TreeCull,
  /// Filling streamed grass tiles near the camera (compute).
  GrassGenerate,
  /// Grass culling and thinning (compute).
  GrassCull,
  /// Filling streamed boulder tiles near the camera (compute).
  BoulderGenerate,
  /// Boulder culling and levels of detail (compute).
  BoulderCull,
  /// Tree shadow map.
  TreeShadow,
  /// Boulders cast into the tree shadow map.
  BoulderShadow,
  /// Terrain.
  Terrain,
  /// The canopy layer over distant forests.
  Canopy,
  /// Bank strips beside streams narrower than a heightmap sample.
  BankStrips,
  /// Full tree meshes near the camera.
  TreeMesh,
  /// Lighter tree meshes from 50 m.
  TreeMeshLight,
  /// Tree impostors.
  TreeImpostor,
  /// Grass tufts.
  Grass,
  /// Boulders and talus.
  Boulders,
  /// Quarter-resolution clouds, while distant clouds are reused.
  QuarterClouds,
  /// Clouds.
  Clouds,
  /// Sky, fog and tone mapping.
  Composite,
  /// The half-resolution scene copy water reflects.
  SceneCopy,
  /// The ocean with sea ice.
  SeaIceOcean,
  /// The ocean without sea ice.
  OpenOcean,
  /// Rivers, lakes and plunge pools.
  InlandWater,
  /// Waterfall sheets and mist.
  Falls,
  /// The final pass: upscaling and lens drops.
  Present,
}

impl PipelineKind {
  /// Every kind, in the order a frame uses them.
  pub const ALL: [Self; 28] = [
    Self::TerrainShadow,
    Self::SurfaceWeather,
    Self::Grounding,
    Self::TreeGenerate,
    Self::TreeCull,
    Self::GrassGenerate,
    Self::GrassCull,
    Self::BoulderGenerate,
    Self::BoulderCull,
    Self::TreeShadow,
    Self::BoulderShadow,
    Self::Terrain,
    Self::Canopy,
    Self::BankStrips,
    Self::TreeMesh,
    Self::TreeMeshLight,
    Self::TreeImpostor,
    Self::Grass,
    Self::Boulders,
    Self::QuarterClouds,
    Self::Clouds,
    Self::Composite,
    Self::SceneCopy,
    Self::SeaIceOcean,
    Self::OpenOcean,
    Self::InlandWater,
    Self::Falls,
    Self::Present,
  ];

  fn slot(self) -> usize {
    Self::ALL
      .iter()
      .position(|kind| *kind == self)
      .unwrap_or_default()
  }
}

/// What a scene draws, from state the engine already has. Each field
/// decides which pipelines exist; see [`Needs::wants`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Needs {
  /// Terrain materials the ground uses, one bit per material. Their
  /// texture layers are baked when first used.
  pub terrain_materials: u32,
  /// Terrain sun shadows are on.
  pub terrain_shadows: bool,
  /// The weather wets and snows on the ground.
  pub surface_weather: bool,
  /// Trees are present.
  pub trees: bool,
  /// Trees near the camera are drawn as full meshes.
  pub tree_meshes: bool,
  /// Trees from 50 m are drawn with their own, lighter pipeline. Until it
  /// exists, the full meshes' pipeline draws them.
  pub tree_light_meshes: bool,
  /// Trees cast shadows.
  pub tree_shadows: bool,
  /// Grass is present: tufts, reeds or both.
  pub grass: bool,
  /// Trees stream in tiles around the camera.
  pub tree_tiles: bool,
  /// Grass streams in tiles around the camera.
  pub grass_tiles: bool,
  /// Boulders are on and in view: tiles with boulders lie within the
  /// boulder distance, in the frustum.
  pub boulders: bool,
  /// The canopy layer is in view: trees stream and the view reaches past
  /// 0.8 of the canopy distance.
  pub canopy: bool,
  /// Clouds are on.
  pub clouds: bool,
  /// The 3D cloud noise is sampled: by clouds, or by volumetric mist. It
  /// is baked when first needed.
  pub cloud_noise: bool,
  /// Distant clouds may be reused between frames.
  pub cloud_reuse: bool,
  /// Water is drawn.
  pub water: bool,
  /// The sea can freeze.
  pub sea_ice: bool,
  /// The sea is near freezing, so either ocean pipeline may be drawn.
  pub sea_near_freezing: bool,
  /// Rivers, lakes or plunge pools are present.
  pub inland_water: bool,
  /// Waterfalls are present.
  pub falls: bool,
  /// Bank strips are present.
  pub bank_strips: bool,
  /// Water reflects the scene on screen.
  pub reflections: bool,
  /// The frame goes through the final pass: lens drops, or a scene drawn
  /// below the canvas resolution.
  pub present: bool,
}

impl Needs {
  /// Whether a pipeline is needed. The terrain and the composite are
  /// always needed.
  pub fn wants(&self, kind: PipelineKind) -> bool {
    match kind {
      PipelineKind::TerrainShadow => self.terrain_shadows,
      PipelineKind::SurfaceWeather => self.surface_weather,
      PipelineKind::TreeCull | PipelineKind::TreeImpostor => self.trees,
      PipelineKind::TreeShadow => self.trees && self.tree_shadows,
      PipelineKind::TreeMesh => self.trees && self.tree_meshes,
      PipelineKind::TreeMeshLight => self.trees && self.tree_meshes && self.tree_light_meshes,
      PipelineKind::Terrain | PipelineKind::Composite => true,
      PipelineKind::Grounding => self.trees || self.grass || self.boulders,
      PipelineKind::BoulderGenerate | PipelineKind::BoulderCull | PipelineKind::Boulders => {
        self.boulders
      }
      PipelineKind::BoulderShadow => self.boulders && self.tree_shadows,
      PipelineKind::Grass => self.grass,
      PipelineKind::TreeGenerate => self.tree_tiles,
      PipelineKind::GrassGenerate | PipelineKind::GrassCull => self.grass_tiles,
      PipelineKind::Canopy => self.canopy,
      PipelineKind::QuarterClouds => self.clouds && self.cloud_reuse,
      PipelineKind::Clouds => self.clouds,
      PipelineKind::SeaIceOcean => self.water && (self.sea_ice || self.sea_near_freezing),
      PipelineKind::OpenOcean => self.water && (!self.sea_ice || self.sea_near_freezing),
      PipelineKind::InlandWater => self.water && self.inland_water,
      PipelineKind::Falls => self.water && self.falls,
      PipelineKind::BankStrips => self.water && self.bank_strips,
      PipelineKind::SceneCopy => self.water && self.reflections,
      PipelineKind::Present => self.present,
    }
  }
}

/// Pipelines created on demand, one slot per [`PipelineKind`].
#[derive(Debug)]
pub struct PipelineSlots<T> {
  slots: Vec<Option<T>>,
}

impl<T> Default for PipelineSlots<T> {
  fn default() -> Self {
    Self {
      slots: PipelineKind::ALL.iter().map(|_| None).collect(),
    }
  }
}

impl<T> PipelineSlots<T> {
  /// The pipeline of a kind, if it has been created.
  pub fn get(&self, kind: PipelineKind) -> Option<&T> {
    self.slots[kind.slot()].as_ref()
  }

  /// Create every pipeline `needs` wants that does not exist yet, in the
  /// order a frame uses them. Pipelines no longer needed are kept, so
  /// switching a system back on never compiles it again. Returns how many
  /// were created.
  pub fn ensure(&mut self, needs: &Needs, mut create: impl FnMut(PipelineKind) -> T) -> usize {
    let mut created = 0;

    for kind in PipelineKind::ALL {
      if needs.wants(kind) && self.slots[kind.slot()].is_none() {
        self.slots[kind.slot()] = Some(create(kind));
        created += 1;
      }
    }

    created
  }

  /// Create at most one pipeline that `likely` wants and that does not
  /// exist yet: the warm-up after the first frame, so a later switch (to
  /// rain, or to a freezing sea) does not stall a frame. Returns the kind
  /// created, if any.
  pub fn warm_one(
    &mut self,
    likely: &Needs,
    create: impl FnOnce(PipelineKind) -> T,
  ) -> Option<PipelineKind> {
    let kind = PipelineKind::ALL
      .into_iter()
      .find(|kind| likely.wants(*kind) && self.slots[kind.slot()].is_none())?;
    self.slots[kind.slot()] = Some(create(kind));
    Some(kind)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn pipelines_are_created_once_in_frame_order() {
    let mut slots = PipelineSlots::<PipelineKind>::default();
    let needs = Needs {
      trees: true,
      water: true,
      inland_water: true,
      ..Needs::default()
    };
    let mut order = Vec::new();
    let created = slots.ensure(&needs, |kind| {
      order.push(kind);
      kind
    });

    assert_eq!(created, order.len());
    assert_eq!(
      order,
      [
        PipelineKind::Grounding,
        PipelineKind::TreeCull,
        PipelineKind::Terrain,
        PipelineKind::TreeImpostor,
        PipelineKind::Composite,
        PipelineKind::OpenOcean,
        PipelineKind::InlandWater,
      ]
    );
    assert_eq!(slots.ensure(&needs, |kind| kind), 0);
    assert_eq!(
      slots.get(PipelineKind::Terrain),
      Some(&PipelineKind::Terrain)
    );
    assert!(slots.get(PipelineKind::Grass).is_none());
  }

  #[test]
  fn boulders_in_view_bring_their_passes_and_their_shadows_with_tree_shadows() {
    let boulders = Needs {
      boulders: true,
      ..Needs::default()
    };
    let wanted: Vec<PipelineKind> = PipelineKind::ALL
      .into_iter()
      .filter(|kind| boulders.wants(*kind))
      .collect();
    assert_eq!(
      wanted,
      [
        PipelineKind::Grounding,
        PipelineKind::BoulderGenerate,
        PipelineKind::BoulderCull,
        PipelineKind::Terrain,
        PipelineKind::Boulders,
        PipelineKind::Composite,
      ]
    );
    assert!(Needs {
      tree_shadows: true,
      ..boulders
    }
    .wants(PipelineKind::BoulderShadow));
    assert!(!Needs {
      tree_shadows: true,
      ..Needs::default()
    }
    .wants(PipelineKind::BoulderShadow));
  }

  #[test]
  fn the_warm_up_creates_one_pipeline_at_a_time() {
    let mut slots = PipelineSlots::<()>::default();
    let likely = Needs {
      water: true,
      sea_ice: true,
      present: true,
      ..Needs::default()
    };
    slots.ensure(&Needs::default(), |_| ());

    assert_eq!(
      slots.warm_one(&likely, |_| ()),
      Some(PipelineKind::SeaIceOcean)
    );
    assert_eq!(slots.warm_one(&likely, |_| ()), Some(PipelineKind::Present));
    assert_eq!(slots.warm_one(&likely, |_| ()), None);
  }
}
