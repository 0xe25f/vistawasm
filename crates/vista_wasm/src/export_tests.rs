//! Map and tree export against the engine's own data.

use vista_types::{
  BiomeKind, FloraOptions, FractalTerrainOptions, RawHeightmapOptions, RawSampleFormat,
  VistaEngineOptions,
};

use crate::engine::EngineCore;
use crate::export::{MapData, MapKind, TREE_RECORD_FLOATS};

const KINDS: [MapKind; 14] = [
  MapKind::Height,
  MapKind::Biome,
  MapKind::Water,
  MapKind::WaterDepth,
  MapKind::Flow,
  MapKind::Discharge,
  MapKind::Materials,
  MapKind::Slope,
  MapKind::Normals,
  MapKind::Occlusion,
  MapKind::Temperature,
  MapKind::Moisture,
  MapKind::TreeDensity,
  MapKind::GrassDensity,
];

fn island(flora: FloraOptions) -> EngineCore {
  let mut engine = EngineCore::new_for_tests(VistaEngineOptions {
    flora: Some(flora),
    ..VistaEngineOptions::default()
  })
  .unwrap();
  let options = FractalTerrainOptions {
    seed: 4_242,
    size: 128,
    horizontal_scale_metres: 24.0,
    shape: Some(vista_types::TerrainShapeOptions {
      island: Some(0.5),
      ..Default::default()
    }),
    ..FractalTerrainOptions::default()
  };
  futures_executor::block_on(engine.generate_fractal(options)).unwrap();
  engine
}

fn forest() -> EngineCore {
  island(FloraOptions {
    density: 2.0,
    ..FloraOptions::default()
  })
}

fn floats(data: &MapData) -> &[f32] {
  match data {
    MapData::F32(values) => values,
    MapData::U8(_) => panic!("expected a float map"),
  }
}

fn bytes(data: &MapData) -> &[u8] {
  match data {
    MapData::U8(values) => values,
    MapData::F32(_) => panic!("expected a byte map"),
  }
}

#[test]
fn every_map_holds_width_x_height_x_channels_values_of_its_type() {
  let engine = forest();

  for kind in KINDS {
    for size in [None, Some([37, 53])] {
      let map = engine.export_map(kind, size).unwrap();
      let [width, height] = size.unwrap_or([128, 128]);
      assert_eq!((map.width, map.height), (width, height), "{kind:?}");
      assert_eq!(map.channels, kind.channels());
      let expected = (width * height * kind.channels()) as usize;

      match &map.data {
        MapData::F32(values) => {
          assert!(kind.is_float(), "{kind:?}");
          assert_eq!(values.len(), expected, "{kind:?}");
          assert!(values.iter().all(|value| value.is_finite()), "{kind:?}");
        }
        MapData::U8(values) => {
          assert!(!kind.is_float(), "{kind:?}");
          assert_eq!(values.len(), expected, "{kind:?}");
        }
      }

      let pixel = map.encoding.metres_per_pixel[0];
      assert!((pixel - 127.0 * 24.0 / (width - 1) as f32).abs() < 1e-3);
      assert_eq!(map.encoding.generator, "vistawasm-fractal-0.2.0");
    }
  }
}

#[test]
fn the_biome_and_water_maps_carry_their_legends() {
  let engine = forest();
  let biome = engine.export_map(MapKind::Biome, None).unwrap();
  let legend = biome.encoding.legend.unwrap();
  assert_eq!(legend.len(), BiomeKind::ALL.len());
  assert!(bytes(&biome.data)
    .iter()
    .all(|value| (*value as usize) < legend.len()));
  let water = engine.export_map(MapKind::Water, None).unwrap();
  assert_eq!(water.encoding.legend.unwrap()[3].0, "ocean");
  let materials = engine.export_map(MapKind::Materials, None).unwrap();
  let names = materials.encoding.legend.unwrap();
  assert_eq!(names.len(), 12);
  assert!(!names.iter().any(|(name, _)| name.contains("reserved")));
}

#[test]
fn the_height_map_is_the_exported_heightmap_at_its_own_size() {
  let engine = forest();
  let map = engine.export_map(MapKind::Height, None).unwrap();
  let bytes = engine.export_heightmap().unwrap();
  let heights: Vec<f32> = bytes
    .chunks_exact(4)
    .map(|word| f32::from_le_bytes([word[0], word[1], word[2], word[3]]))
    .collect();
  assert_eq!(floats(&map.data), heights.as_slice());
  let range = map.encoding.range.unwrap();
  assert_eq!(range[0], heights.iter().copied().fold(f32::MAX, f32::min));
  assert_eq!(range[1], heights.iter().copied().fold(f32::MIN, f32::max));
}

#[test]
fn material_weights_sum_to_255() {
  let engine = forest();

  for size in [None, Some([200, 90])] {
    let map = engine.export_map(MapKind::Materials, size).unwrap();

    for (index, weights) in bytes(&map.data).chunks_exact(12).enumerate() {
      let sum: i32 = weights.iter().map(|weight| i32::from(*weight)).sum();
      assert!(
        (sum - 255).abs() <= 2,
        "{size:?} pixel {index}: {weights:?}"
      );
    }
  }
}

#[test]
fn nearest_resampling_keeps_only_biomes_the_map_has() {
  let engine = forest();
  let native = engine.export_map(MapKind::Biome, None).unwrap();
  let present: std::collections::HashSet<u8> = bytes(&native.data).iter().copied().collect();

  for size in [[300, 300], [17, 91], [2, 2]] {
    let map = engine.export_map(MapKind::Biome, Some(size)).unwrap();
    assert!(bytes(&map.data).iter().all(|value| present.contains(value)));
  }
}

/// A plane rising at `degrees` along +x, as a raw heightmap.
fn plane(degrees: f32) -> EngineCore {
  let mut engine = EngineCore::new_for_tests(VistaEngineOptions::default()).unwrap();
  let (size, metres) = (64u32, 10.0f32);
  let rise = degrees.to_radians().tan() * metres;
  let bytes: Vec<u8> = (0..size * size)
    .flat_map(|index| (100.0 + (index % size) as f32 * rise).to_le_bytes())
    .collect();
  let options = RawHeightmapOptions {
    width: size,
    height: size,
    sample_format: RawSampleFormat::Float32,
    byte_order: None,
    metres_per_sample: metres,
    height_scale_metres: 1.0,
    no_data_value: None,
    sea_level_metres: Some(0.0),
    landform: None,
  };
  futures_executor::block_on(engine.load_raw_heightmap(&bytes, options)).unwrap();
  engine
}

#[test]
fn a_thirty_degree_plane_exports_thirty_degrees_and_unit_normals() {
  let engine = plane(30.0);
  let slope = engine.export_map(MapKind::Slope, None).unwrap();

  // Edge samples see one neighbour, so only the inside is a full slope.
  for z in 1..63 {
    for x in 1..63 {
      let value = floats(&slope.data)[z * 64 + x];
      assert!((value - 30.0).abs() <= 0.5, "({x}, {z}): {value}");
    }
  }

  for size in [None, Some([150, 70])] {
    let normals = engine.export_map(MapKind::Normals, size).unwrap();

    for normal in floats(&normals.data).chunks_exact(3) {
      let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
      assert!((length - 1.0).abs() <= 1e-3, "{normal:?}");
    }
  }
}

#[test]
fn water_lies_where_the_ground_is_below_it() {
  let engine = forest();
  let height = engine.export_map(MapKind::Height, None).unwrap();
  let water = engine.export_map(MapKind::Water, None).unwrap();
  let depth = engine.export_map(MapKind::WaterDepth, None).unwrap();
  let (height, water, depth) = (
    floats(&height.data),
    bytes(&water.data),
    floats(&depth.data),
  );
  let mut kinds = [0; 7];

  for index in 0..water.len() {
    kinds[water[index] as usize] += 1;

    match water[index] {
      0 => assert_eq!(depth[index], 0.0),
      3 => {
        assert!(height[index] < 0.0);
        assert!((depth[index] + height[index]).abs() < 1e-3);
      }
      _ => assert!(depth[index] >= 0.0),
    }
  }

  // An island in the sea, with rivers running down to it.
  assert!(kinds[3] > 1000, "{kinds:?}");
  assert!(kinds[1] + kinds[2] + kinds[4] > 0, "{kinds:?}");
}

#[test]
fn density_maps_follow_the_vegetation_settings() {
  let mut engine = forest();
  let trees = engine.export_map(MapKind::TreeDensity, None).unwrap();
  let grass = engine.export_map(MapKind::GrassDensity, None).unwrap();
  assert!(bytes(&trees.data).iter().any(|value| *value > 0));
  assert!(bytes(&grass.data).iter().any(|value| *value > 100));
  assert_eq!(trees.encoding.scale, Some(4.0));

  engine
    .set_flora(FloraOptions {
      enabled: false,
      ..FloraOptions::default()
    })
    .unwrap();
  engine
    .set_grass(vista_types::GrassOptions {
      enabled: false,
      ..Default::default()
    })
    .unwrap();
  let trees = engine.export_map(MapKind::TreeDensity, None).unwrap();
  let grass = engine.export_map(MapKind::GrassDensity, None).unwrap();
  assert!(bytes(&trees.data).iter().all(|value| *value == 0));
  assert!(bytes(&grass.data).iter().all(|value| *value == 0));
}

#[test]
fn export_sizes_are_validated() {
  let engine = forest();

  for size in [[1, 10], [10, 1], [2049, 2], [4096, 4096], [0, 0]] {
    let error = engine.export_map(MapKind::Height, Some(size)).unwrap_err();
    assert!(error.to_string().contains("from 2 to 2048"), "{error}");
  }

  let empty = EngineCore::new_for_tests(VistaEngineOptions::default()).unwrap();
  assert!(empty.export_map(MapKind::Height, None).is_err());
  assert!(empty.export_trees(None, None).is_err());
}

/// The ground at full detail under world `(x, z)`, from the exported
/// heights.
fn ground_at(engine: &EngineCore, x: f32, z: f32) -> f32 {
  let map = engine.export_map(MapKind::Height, None).unwrap();
  let metadata = vista_types::TerrainMetadata {
    width: map.width,
    height: map.height,
    metres_per_sample: map.encoding.metres_per_pixel[0],
    ..Default::default()
  };
  let (width, height) = (map.width, map.height);
  let heights = floats(&map.data).to_vec();
  let count = heights.len();
  let terrain =
    crate::terrain::HeightMap::from_values(width, height, heights, vec![false; count], metadata)
      .unwrap();
  let (sx, sz) = crate::render::terrain_mesh::world_to_sample_coordinates(&terrain, x, z);
  crate::render::terrain_mesh::full_detail_height(&terrain, sx, sz)
}

type Record = [u32; 7];

/// The parts of a tree the renderer and the export share: position
/// across the ground, species, variant, scale, rotation, tint and dryness.
fn record(tree: &[f32]) -> Record {
  [0, 2, 3, 5, 6, 7, 8].map(|at| tree[at].to_bits())
}

#[test]
fn exported_trees_are_the_lattice_the_renderer_places() {
  let engine = forest();
  let region = [-600.0, -400.0, 300.0, 500.0];
  let packed = engine.export_trees(Some(region), None).unwrap();
  assert_eq!(packed.len() % TREE_RECORD_FLOATS, 0);
  assert_eq!(engine.export_trees(Some(region), None).unwrap(), packed);
  let mut exported: Vec<Record> = packed
    .chunks_exact(TREE_RECORD_FLOATS)
    .map(record)
    .collect();

  // Native builds place every tree of the lattice, far and streamed
  // alike (the far set's share is 1 there).
  let mut placed: Vec<Record> = engine
    .placed_trees()
    .iter()
    .filter(|tree| {
      let [x, _, z] = tree.position;
      x >= region[0] && x <= region[2] && z >= region[1] && z <= region[3]
    })
    .map(|tree| {
      record(&[
        tree.position[0],
        0.0,
        tree.position[2],
        tree.species_index() as f32,
        0.0,
        tree.scale,
        tree.rotation,
        tree.tint,
        tree.dryness,
      ])
    })
    .collect();
  assert!(placed.len() > 100, "{} trees", placed.len());
  exported.sort_unstable();
  placed.sort_unstable();
  assert_eq!(exported, placed);

  let variants = FloraOptions::default().variants_per_species as f32;

  for tree in packed.chunks_exact(TREE_RECORD_FLOATS) {
    assert!(tree[4] < variants && tree[4].fract() == 0.0);
    assert_eq!(tree[9], 0.0);
    // Grounded by its roots: never above the ground at its trunk.
    let (x, z) = (tree[0], tree[2]);
    assert!(tree[1] <= ground_at(&engine, x, z) + 1e-3);
  }

  // The whole map holds more, and too low a cap is an error that says
  // how to export less.
  let all = engine.export_trees(None, None).unwrap();
  assert!(all.len() > packed.len());
  let count = (packed.len() / TREE_RECORD_FLOATS) as u32;
  let error = engine
    .export_trees(Some(region), Some(count - 1))
    .unwrap_err()
    .to_string();
  assert!(error.contains("region"), "{error}");
  assert_eq!(
    engine
      .export_trees(Some(region), Some(count))
      .unwrap()
      .len(),
    packed.len()
  );
}

#[test]
fn hand_placed_trees_are_exported_as_such() {
  use crate::render::flora::{TreeInstance, TREE_GROUNDED};

  let mut engine = forest();
  let tree = |x: f32, species: u32, flags: u32| TreeInstance {
    position: [x, 5_000.0, 10.0],
    scale: 1.5,
    rotation: 0.25,
    tint: 0.5,
    species: species | flags,
    dryness: 0.1,
  };
  engine
    .set_tree_instances(Some(vec![
      tree(0.0, 2, TREE_GROUNDED),
      tree(40.0, 4, 0),
      tree(5_000.0, 1, 0),
    ]))
    .unwrap();
  let packed = engine
    .export_trees(Some([-100.0, -100.0, 100.0, 100.0]), None)
    .unwrap();
  assert_eq!(packed.len(), 2 * TREE_RECORD_FLOATS);
  assert_eq!(&packed[3..5], &[2.0, 0.0]);
  assert_eq!(packed[9], 1.0);
  // The grounded one stands on the ground, the other where it was put.
  assert!(packed[1] <= ground_at(&engine, 0.0, 10.0), "{}", packed[1]);
  assert_eq!(packed[TREE_RECORD_FLOATS + 1], 5_000.0);
  assert_eq!(
    engine.export_trees(None, None).unwrap().len(),
    3 * TREE_RECORD_FLOATS
  );
}
