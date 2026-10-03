//! Painted biomes, vegetation masks, source heights and bundle round
//! trips against the engine.

use vista_types::{
  BiomeKind, BiomeOptions, FloraOptions, FractalTerrainOptions, RawHeightmapOptions,
  RawSampleFormat, VistaEngineOptions, WaterMask,
};

use crate::engine::EngineCore;
use crate::export::{MapData, MapKind};
use crate::terrain::painted::{density_byte, density_multiplier, PaintedBiomes, NOT_PAINTED};

const SIZE: u32 = 128;

fn options(flora: f32) -> VistaEngineOptions {
  VistaEngineOptions {
    flora: Some(FloraOptions {
      density: flora,
      ..FloraOptions::default()
    }),
    ..VistaEngineOptions::default()
  }
}

fn island(options: VistaEngineOptions) -> EngineCore {
  let mut engine = EngineCore::new_for_tests(options).unwrap();
  let terrain = FractalTerrainOptions {
    seed: 4_242,
    size: SIZE,
    horizontal_scale_metres: 24.0,
    shape: Some(vista_types::TerrainShapeOptions {
      island: Some(0.5),
      ..Default::default()
    }),
    ..FractalTerrainOptions::default()
  };
  futures_executor::block_on(engine.generate_fractal(terrain)).unwrap();
  engine
}

fn bytes(map: &MapData) -> &[u8] {
  match map {
    MapData::U8(values) => values,
    MapData::F32(_) => panic!("expected a byte map"),
  }
}

fn floats(map: &MapData) -> &[f32] {
  match map {
    MapData::F32(values) => values,
    MapData::U8(_) => panic!("expected a float map"),
  }
}

/// A map's values as numbers, whatever their type.
fn values(map: &MapData) -> Vec<f32> {
  match map {
    MapData::F32(values) => values.clone(),
    MapData::U8(values) => values.iter().map(|value| f32::from(*value)).collect(),
  }
}

fn export(engine: &EngineCore, kind: MapKind) -> MapData {
  engine.export_map(kind, None).unwrap().data
}

/// Three regions: forest on the west, savannah in the north-east and
/// meadow in the south-east.
fn three_regions() -> Vec<u8> {
  (0..SIZE * SIZE)
    .map(|index| {
      let (x, y) = (index % SIZE, index / SIZE);

      if x < SIZE / 2 {
        BiomeKind::InnerForest as u8
      } else if y < SIZE / 2 {
        BiomeKind::SavannahExpanse as u8
      } else {
        BiomeKind::GrassyMeadows as u8
      }
    })
    .collect()
}

#[test]
fn painted_interiors_keep_their_biome_and_only_borders_vary() {
  let painted = three_regions();
  let border = 4;
  let map = PaintedBiomes::new((SIZE, SIZE, &painted), border, (SIZE, SIZE), 99)
    .unwrap()
    .0;
  let mut changed_by_the_warp = 0;

  for y in 0..SIZE {
    for x in 0..SIZE {
      let here = painted[(y * SIZE + x) as usize];
      let got = map.at(x, y).unwrap() as u8;
      // Distance to the nearest painted edge, axis by axis.
      let near_edge = (0..SIZE).any(|oy| {
        (0..SIZE).any(|ox| {
          x.abs_diff(ox) <= border
            && y.abs_diff(oy) <= border
            && painted[(oy * SIZE + ox) as usize] != here
        })
      });

      if !near_edge {
        assert_eq!(got, here, "interior sample ({x}, {y}) changed");
      } else if got != here {
        changed_by_the_warp += 1;
      }
    }
  }

  // Irregular: the warp moves the border, but not everywhere alike.
  assert!(changed_by_the_warp > 50, "{changed_by_the_warp}");

  // Deterministic.
  let again = PaintedBiomes::new((SIZE, SIZE, &painted), border, (SIZE, SIZE), 99)
    .unwrap()
    .0;
  assert_eq!(map, again);

  for y in 0..SIZE {
    for x in 0..SIZE {
      assert_eq!(map.at(x, y), again.at(x, y));
    }
  }
}

#[test]
fn the_engine_keeps_every_painted_biome_but_ocean_above_the_sea() {
  let mut engine = island(options(1.0));
  let height = export(&engine, MapKind::Height);
  let height = floats(&height);
  let sea = engine
    .export_map(MapKind::Height, None)
    .unwrap()
    .encoding
    .sea_level_metres;
  let mut painted = three_regions();

  // A band of ocean across the middle, over land and sea alike.
  for x in 0..SIZE {
    painted[((SIZE / 2) * SIZE + x) as usize] = BiomeKind::Ocean as u8;
  }

  let warnings = engine
    .set_biome_map(Some((SIZE, SIZE, &painted)), 0)
    .unwrap();
  let biome = export(&engine, MapKind::Biome);
  let biome = bytes(&biome);
  let mut fell_back = 0;

  for index in 0..painted.len() {
    if painted[index] == BiomeKind::Ocean as u8 && height[index] >= sea - 0.3 {
      assert_ne!(biome[index], BiomeKind::Ocean as u8);
      fell_back += 1;
    } else {
      assert_eq!(biome[index], painted[index], "sample {index}");
    }
  }

  assert!(fell_back > 0);
  assert_eq!(warnings.len(), 1, "{warnings:?}");
  assert!(
    warnings[0].starts_with(&format!("{fell_back} samples painted as ocean")),
    "{}",
    warnings[0]
  );

  // Clearing classifies the terrain itself again.
  let automatic = island(options(1.0));
  engine.set_biome_map(None, 0).unwrap();
  assert_eq!(
    export(&engine, MapKind::Biome),
    export(&automatic, MapKind::Biome)
  );
}

#[test]
fn unpainted_samples_are_classified_as_usual() {
  let mut engine = island(options(1.0));
  let automatic = export(&engine, MapKind::Biome);
  let mut painted = vec![NOT_PAINTED; (SIZE * SIZE) as usize];

  for y in 0..SIZE / 4 {
    for x in 0..SIZE / 4 {
      painted[(y * SIZE + x) as usize] = BiomeKind::SwampWetlands as u8;
    }
  }

  engine
    .set_biome_map(Some((SIZE, SIZE, &painted)), 3)
    .unwrap();
  let biome = export(&engine, MapKind::Biome);

  for y in 0..SIZE {
    for x in 0..SIZE {
      let index = (y * SIZE + x) as usize;

      if x > SIZE / 4 + 3 || y > SIZE / 4 + 3 {
        assert_eq!(bytes(&biome)[index], bytes(&automatic)[index], "({x}, {y})");
      } else if x + 3 < SIZE / 4 && y + 3 < SIZE / 4 {
        assert_eq!(bytes(&biome)[index], BiomeKind::SwampWetlands as u8);
      }
    }
  }
}

#[test]
fn materials_blend_across_painted_borders() {
  let mut engine = island(options(1.0));
  let painted: Vec<u8> = (0..SIZE * SIZE)
    .map(|index| {
      if index % SIZE < SIZE / 2 {
        BiomeKind::InnerForest as u8
      } else {
        BiomeKind::SavannahExpanse as u8
      }
    })
    .collect();
  let materials = |engine: &mut EngineCore, border| {
    engine
      .set_biome_map(Some((SIZE, SIZE, &painted)), border)
      .unwrap();
    bytes(&export(engine, MapKind::Materials)).to_vec()
  };
  let hard = materials(&mut engine, 0);
  let soft = materials(&mut engine, 8);
  let changed = |from: u32, to: u32| {
    (0..SIZE)
      .flat_map(|y| (from..to).map(move |x| (y * SIZE + x) as usize))
      .filter(|index| hard[index * 12..index * 12 + 12] != soft[index * 12..index * 12 + 12])
      .count()
  };

  // Within the warp and blend of the border the ground changes; far from
  // it, it does not.
  assert!(changed(SIZE / 2 - 6, SIZE / 2 + 6) > 100);
  assert_eq!(changed(0, SIZE / 2 - 13), 0);
  assert_eq!(changed(SIZE / 2 + 13, SIZE), 0);
}

#[test]
fn painted_ice_forms_glacier_even_in_a_warm_climate() {
  let mut engine = island(VistaEngineOptions {
    biomes: Some(BiomeOptions {
      mean_temperature_celsius: Some(20.0),
      ..BiomeOptions::default()
    }),
    ..options(1.0)
  });
  let before = export(&engine, MapKind::Height);
  let painted = vec![BiomeKind::IceArctic as u8; (SIZE * SIZE) as usize];
  engine
    .set_biome_map(Some((SIZE, SIZE, &painted)), 0)
    .unwrap();
  let materials = export(&engine, MapKind::Materials);
  let biome = export(&engine, MapKind::Biome);
  let icy = bytes(&biome)
    .iter()
    .enumerate()
    .filter(|(index, biome)| {
      **biome == BiomeKind::IceArctic as u8 && {
        let weights = &bytes(&materials)[index * 12..index * 12 + 12];
        u32::from(weights[5]) + u32::from(weights[8]) > 128
      }
    })
    .count();

  assert!(icy > 1000, "{icy}");
  // The ice is smoothed into glaciers, which raises the heights.
  assert_ne!(export(&engine, MapKind::Height), before);
}

#[test]
fn biome_maps_are_validated() {
  let mut engine = island(options(1.0));
  let data = vec![0u8; 16];

  for (map, border, message) in [
    ((1, 16, &data[..]), 0, "from 2 to 2048"),
    ((4, 5, &data[..]), 0, "must hold width x height = 20"),
    ((4, 4, &[19u8; 16][..]), 0, "holds 19"),
    ((4, 4, &data[..]), 9, "borderSamples must be from 0 to 8"),
  ] {
    let error = engine.set_biome_map(Some(map), border).unwrap_err();
    assert!(error.to_string().contains(message), "{error}");
  }

  let warnings = engine.set_biome_map(Some((4, 4, &data)), 3).unwrap();
  assert!(warnings[0].contains("resampled to fit"), "{warnings:?}");
}

#[test]
fn density_masks_map_128_to_no_change_and_255_to_double() {
  assert_eq!(density_multiplier(0), 0.0);
  assert_eq!(density_multiplier(64), 0.5);
  assert_eq!(density_multiplier(128), 1.0);
  assert_eq!(density_multiplier(255), 2.0);

  for value in 0..=255u8 {
    assert_eq!(density_byte(density_multiplier(value)), value);
  }
}

fn trees(engine: &EngineCore) -> usize {
  engine.export_trees(None, None).unwrap().len() / crate::export::TREE_RECORD_FLOATS
}

fn tree_density(engine: &EngineCore) -> f32 {
  values(&export(engine, MapKind::TreeDensity)).iter().sum()
}

#[test]
fn a_tree_mask_of_zero_clears_trees_and_255_doubles_them_capped() {
  let mut engine = island(options(1.0));
  let (count, density) = (trees(&engine), tree_density(&engine));
  assert!(count > 100, "{count}");

  let size = (SIZE * SIZE) as usize;
  engine
    .set_vegetation_mask(false, Some((SIZE, SIZE, &vec![0; size])))
    .unwrap();
  assert_eq!(trees(&engine), 0);
  assert_eq!(tree_density(&engine), 0.0);

  engine
    .set_vegetation_mask(false, Some((SIZE, SIZE, &vec![255; size])))
    .unwrap();
  let doubled = tree_density(&engine);
  // Twice the share, less where the cover texture's share is at its most.
  assert!(
    doubled > density * 1.6 && doubled <= density * 2.0 + 1.0,
    "{density} {doubled}"
  );

  // At density 3 doubling would pass density 4, so the mask is capped.
  let mut dense = island(options(3.0));
  let full = tree_density(&dense);
  dense
    .set_vegetation_mask(false, Some((SIZE, SIZE, &vec![255; size])))
    .unwrap();
  let capped = tree_density(&dense);
  let most =
    crate::render::lattice::target_density(4.0) / crate::render::lattice::target_density(3.0);
  assert!(
    capped <= full * most + 1.0 && capped > full * 1.4,
    "{full} {capped}"
  );

  // Clearing restores the trees exactly.
  engine.set_vegetation_mask(false, None).unwrap();
  assert_eq!(trees(&engine), count);
}

#[test]
fn a_tree_clearing_is_empty_and_the_rest_is_unchanged() {
  let mut engine = island(options(2.0));
  let before = values(&export(&engine, MapKind::TreeDensity));
  let mask: Vec<u8> = (0..SIZE * SIZE)
    .map(|index| if index % SIZE < SIZE / 2 { 0 } else { 128 })
    .collect();
  engine
    .set_vegetation_mask(false, Some((SIZE, SIZE, &mask)))
    .unwrap();
  let after = values(&export(&engine, MapKind::TreeDensity));

  for index in 0..after.len() {
    let x = index as u32 % SIZE;

    if x < SIZE / 2 - 2 {
      assert_eq!(after[index], 0.0);
    } else if x > SIZE / 2 + 2 {
      assert_eq!(after[index], before[index]);
    }
  }
}

#[test]
fn a_grass_mask_scales_the_grass_and_is_capped() {
  let mut engine = island(options(1.0));
  let size = (SIZE * SIZE) as usize;
  let grass = |engine: &EngineCore| -> f32 {
    bytes(&export(engine, MapKind::GrassDensity))
      .iter()
      .map(|value| f32::from(*value))
      .sum()
  };
  let before = grass(&engine);

  engine
    .set_vegetation_mask(true, Some((SIZE, SIZE, &vec![0; size])))
    .unwrap();
  assert_eq!(grass(&engine), 0.0);

  engine
    .set_vegetation_mask(true, Some((SIZE, SIZE, &vec![255; size])))
    .unwrap();
  assert!(grass(&engine) > before * 1.2, "{before} {}", grass(&engine));

  // Resampled masks warn.
  let warning = engine
    .set_vegetation_mask(true, Some((4, 4, &[128; 16])))
    .unwrap();
  assert!(warning.unwrap().contains("grass mask is 4 x 4"));
  assert_eq!(grass(&engine), before);
}

fn water_mask() -> WaterMask {
  WaterMask {
    width: SIZE,
    height: SIZE,
    data: (0..SIZE * SIZE)
      .map(|index| {
        let (x, y) = (index % SIZE, index / SIZE);

        if x.abs_diff(40) < 6 && y.abs_diff(70) < 6 {
          200
        } else if y == 90 && (20..100).contains(&x) {
          40
        } else {
          0
        }
      })
      .collect(),
  }
}

#[test]
fn source_heights_undo_rivers_glaciers_and_the_water_mask() {
  let mut engine = island(VistaEngineOptions {
    biomes: Some(BiomeOptions {
      mean_temperature_celsius: Some(-8.0),
      ..BiomeOptions::default()
    }),
    ..options(1.0)
  });
  let loaded = floats(&export(&engine, MapKind::SourceHeight)).to_vec();
  let size = SIZE as usize;
  let bytes: Vec<u8> = loaded
    .iter()
    .flat_map(|height| height.to_le_bytes())
    .collect();
  let raw = RawHeightmapOptions {
    width: SIZE,
    height: SIZE,
    sample_format: RawSampleFormat::Float32,
    byte_order: None,
    metres_per_sample: 24.0,
    height_scale_metres: 1.0,
    no_data_value: None,
    sea_level_metres: Some(0.0),
    landform: None,
  };
  futures_executor::block_on(engine.load_raw_heightmap(&bytes, raw)).unwrap();
  engine.set_water_mask(Some(water_mask())).unwrap();
  let carved = floats(&export(&engine, MapKind::Height)).to_vec();

  // Rivers, the painted water and glacier ice all changed the heights.
  assert_ne!(carved, loaded);
  assert_eq!(carved.len(), size * size);
  assert_eq!(floats(&export(&engine, MapKind::SourceHeight)), &loaded[..]);
}

/// The inputs of a bundle: source heights, painted maps and the options,
/// loaded into a fresh engine as `loadBundle` loads them.
#[test]
fn a_bundle_round_trip_recreates_every_map_and_tree_exactly() {
  let engine_options = options(2.0);
  let mut engine = island(engine_options.clone());
  let mut painted = vec![NOT_PAINTED; (SIZE * SIZE) as usize];

  for y in 0..SIZE / 2 {
    for x in SIZE / 2..SIZE {
      painted[(y * SIZE + x) as usize] = BiomeKind::SavannahExpanse as u8;
    }
  }

  let trees_mask: Vec<u8> = (0..SIZE * SIZE).map(|index| (index % 256) as u8).collect();
  let grass_mask: Vec<u8> = (0..SIZE * SIZE)
    .map(|index| (index / SIZE * 2) as u8)
    .collect();
  engine
    .set_biome_map(Some((SIZE, SIZE, &painted)), 3)
    .unwrap();
  engine.set_water_mask(Some(water_mask())).unwrap();
  engine
    .set_vegetation_mask(false, Some((SIZE, SIZE, &trees_mask)))
    .unwrap();
  engine
    .set_vegetation_mask(true, Some((SIZE, SIZE, &grass_mask)))
    .unwrap();

  let source = floats(&export(&engine, MapKind::SourceHeight)).to_vec();
  let bytes: Vec<u8> = source
    .iter()
    .flat_map(|height| height.to_le_bytes())
    .collect();
  let mut copy = EngineCore::new_for_tests(engine_options).unwrap();
  let raw = RawHeightmapOptions {
    width: SIZE,
    height: SIZE,
    sample_format: RawSampleFormat::Float32,
    byte_order: None,
    metres_per_sample: 24.0,
    height_scale_metres: 1.0,
    no_data_value: None,
    sea_level_metres: Some(0.0),
    landform: Some(Default::default()),
  };
  futures_executor::block_on(copy.load_raw_heightmap(&bytes, raw)).unwrap();
  copy.set_biome_map(Some((SIZE, SIZE, &painted)), 3).unwrap();
  copy.set_water_mask(Some(water_mask())).unwrap();
  copy
    .set_vegetation_mask(false, Some((SIZE, SIZE, &trees_mask)))
    .unwrap();
  copy
    .set_vegetation_mask(true, Some((SIZE, SIZE, &grass_mask)))
    .unwrap();

  for kind in MapKind::ALL {
    assert_eq!(export(&copy, kind), export(&engine, kind), "{kind:?}");
  }

  assert_eq!(
    copy.export_trees(None, None).unwrap(),
    engine.export_trees(None, None).unwrap()
  );
}
