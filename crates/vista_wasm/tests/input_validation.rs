//! Every entry point JavaScript reaches rejects bad numbers, sizes and
//! arrays with a typed error that names the field and its valid range.

use vista_types::{
  AtmosphereOptions, BiomeOptions, CameraOptions, CloudsOptions, DemLoadOptions,
  FractalTerrainOptions, GrassOptions, MistOptions, RawHeightmapOptions, RawSampleFormat,
  RenderQualityOptions, RenderSizeOptions, ShadowOptions, SunOptions, SurfaceOptions,
  TextureTarget, TimeOfDayOptions, TreeSpeciesKind, VistaEngineOptions, VistaErrorCode, WaterMask,
  WaterOptions, WeatherKind, WeatherOptions, WeatherPreset,
};
use vista_wasm::config::VistaEngineConfig;
use vista_wasm::engine::EngineCore;
use vista_wasm::errors::VistaResult;

const BAD: [f32; 3] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];

/// Assert `result` is an options error whose message holds every `needle`.
#[track_caller]
fn rejected<T>(result: VistaResult<T>, needles: &[&str]) {
  let Err(error) = result else {
    panic!("the input should be rejected: {needles:?}");
  };
  let message = error.to_string();
  assert!(
    matches!(
      error.code(),
      VistaErrorCode::OptionsInvalid
        | VistaErrorCode::DemFormatUnsupported
        | VistaErrorCode::DemMetadataMissing
    ),
    "{message}"
  );

  for needle in needles {
    assert!(message.contains(needle), "`{message}` lacks `{needle}`");
  }
}

fn engine() -> EngineCore {
  EngineCore::new_for_tests(VistaEngineOptions::default()).expect("default options")
}

fn engine_with_terrain() -> EngineCore {
  let mut engine = engine();
  let terrain = FractalTerrainOptions {
    size: 32,
    ..FractalTerrainOptions::default()
  };
  futures_executor::block_on(engine.generate_fractal(terrain)).expect("small terrain");
  engine
}

fn create(options: VistaEngineOptions) -> VistaResult<VistaEngineConfig> {
  VistaEngineConfig::from_options(options)
}

#[test]
fn create_and_resize_reject_bad_surfaces() {
  for (width, height) in [(0, 10), (10, 0), (8193, 10), (10, u32::MAX)] {
    rejected(
      create(VistaEngineOptions {
        render: Some(RenderSizeOptions {
          width,
          height,
          device_pixel_ratio: None,
        }),
        ..VistaEngineOptions::default()
      }),
      &["render", "between 1 and 8192"],
    );
    rejected(
      engine().resize(width, height, None),
      &["resize", "between 1 and 8192"],
    );
  }

  for ratio in [0.0, -1.0, 9.0, f32::NAN, f32::INFINITY] {
    rejected(
      engine().resize(100, 100, Some(ratio)),
      &["resize devicePixelRatio", "at most 8"],
    );
  }

  assert!(engine().resize(8192, 8192, Some(8.0)).is_ok());
}

#[test]
fn cameras_reject_bad_numbers() {
  let camera = CameraOptions::default;
  let cases: Vec<(CameraOptions, &str)> = vec![
    (
      CameraOptions {
        position: [f32::NAN, 0.0, 0.0],
        ..camera()
      },
      "camera.position[0]",
    ),
    (
      CameraOptions {
        target: [0.0, 0.0, 1e8],
        ..camera()
      },
      "camera.target[2]",
    ),
    (
      CameraOptions {
        field_of_view_degrees: 0.0,
        ..camera()
      },
      "between 1 and 160",
    ),
    (
      CameraOptions {
        near_metres: Some(-1.0),
        ..camera()
      },
      "camera.nearMetres",
    ),
    (
      CameraOptions {
        far_metres: Some(f32::INFINITY),
        ..camera()
      },
      "camera.farMetres",
    ),
    (
      CameraOptions {
        roll_degrees: Some(f32::NAN),
        ..camera()
      },
      "camera.rollDegrees",
    ),
    (
      CameraOptions {
        minimum_height_above_terrain_metres: Some(-2.0),
        ..camera()
      },
      "camera.minimumHeightAboveTerrainMetres",
    ),
  ];

  for (options, needle) in cases {
    rejected(engine().set_camera(options), &[needle]);
  }
}

#[test]
fn sun_and_atmosphere_are_validated_when_set() {
  for (sun, needle) in [
    (
      SunOptions {
        elevation_degrees: 91.0,
        ..SunOptions::default()
      },
      "sun.elevationDegrees must be between -90 and 90",
    ),
    (
      SunOptions {
        intensity: 0.0,
        ..SunOptions::default()
      },
      "sun.intensity must be greater than 0 and at most 100",
    ),
    (
      SunOptions {
        azimuth_degrees: f32::NAN,
        ..SunOptions::default()
      },
      "sun.azimuthDegrees",
    ),
  ] {
    rejected(engine().set_sun(sun.clone()), &[needle]);
    rejected(
      create(VistaEngineOptions {
        sun: Some(sun),
        ..VistaEngineOptions::default()
      }),
      &[needle],
    );
  }

  let atmosphere = AtmosphereOptions::default;
  for (options, needle) in [
    (
      AtmosphereOptions {
        rayleigh_strength: -1.0,
        ..atmosphere()
      },
      "atmosphere.rayleighStrength",
    ),
    (
      AtmosphereOptions {
        mie_strength: f32::NAN,
        ..atmosphere()
      },
      "atmosphere.mieStrength",
    ),
    (
      AtmosphereOptions {
        haze_distance_metres: 0.0,
        ..atmosphere()
      },
      "atmosphere.hazeDistanceMetres",
    ),
    (
      AtmosphereOptions {
        exposure: 1e6,
        ..atmosphere()
      },
      "atmosphere.exposure",
    ),
    (
      AtmosphereOptions {
        sky_tint: [1.0, 5.0, 1.0],
        ..atmosphere()
      },
      "atmosphere.skyTint",
    ),
  ] {
    rejected(engine().set_atmosphere(options), &[needle]);
  }
}

#[test]
fn documented_unit_ranges_are_enforced() {
  let water = WaterOptions {
    foam: 1.5,
    ..WaterOptions::default()
  };
  rejected(
    engine().set_water(water),
    &["water.foam", "between 0 and 1"],
  );

  let mut water = WaterOptions::default();
  water.waves.steepness = f32::NAN;
  rejected(engine().set_water(water), &["water.waves.steepness"]);

  let water = WaterOptions {
    sea_level_metres: 1e6,
    ..WaterOptions::default()
  };
  rejected(
    engine().set_water(water),
    &["water.seaLevelMetres", "100000"],
  );

  let flora = vista_types::FloraOptions {
    wind_strength: 2.0,
    ..Default::default()
  };
  rejected(engine().set_flora(flora), &["flora.windStrength"]);

  let flora = vista_types::FloraOptions {
    mesh_distance_metres: 1e6,
    ..Default::default()
  };
  rejected(
    engine().set_flora(flora),
    &["flora.meshDistanceMetres", "5000"],
  );

  for view in [0.0, 1_001.0, f32::NAN] {
    let grass = GrassOptions {
      view_distance_metres: view,
      ..GrassOptions::default()
    };
    rejected(
      engine().set_grass(grass),
      &["grass.viewDistanceMetres", "1000"],
    );
  }

  let clouds = CloudsOptions {
    evolution: f32::INFINITY,
    ..CloudsOptions::default()
  };
  rejected(engine().set_clouds(clouds), &["clouds.evolution"]);

  let clouds = CloudsOptions {
    density: 2.0,
    ..CloudsOptions::default()
  };
  rejected(engine().set_clouds(clouds), &["clouds.density"]);

  let mist = MistOptions {
    sun_scattering: -0.1,
    ..MistOptions::default()
  };
  rejected(engine().set_mist(mist), &["mist.sunScattering"]);

  let biomes = BiomeOptions {
    volcanism: 3.0,
    ..BiomeOptions::default()
  };
  rejected(engine().set_biomes(biomes), &["biomes.volcanism"]);

  let mut shadows = ShadowOptions::default();
  shadows.trees.resolution = 300;
  rejected(engine().set_shadows(shadows), &["shadows.trees.resolution"]);

  let surface = SurfaceOptions {
    texture_scale: 0.0,
    ..SurfaceOptions::default()
  };
  rejected(engine().set_surface(surface), &["surface.textureScale"]);
}

#[test]
fn render_quality_rejects_unbounded_values() {
  let quality = RenderQualityOptions::default;
  let cases = [
    (
      RenderQualityOptions {
        max_clipmap_levels: Some(0),
        ..quality()
      },
      "quality.maxClipmapLevels must be between 1 and 12",
    ),
    (
      RenderQualityOptions {
        flora_density_scale: Some(f32::NAN),
        ..quality()
      },
      "quality.floraDensityScale",
    ),
    (
      RenderQualityOptions {
        render_distance_metres: Some(2e9),
        ..quality()
      },
      "quality.renderDistanceMetres",
    ),
    (
      RenderQualityOptions {
        max_frame_rate: Some(5_000.0),
        ..quality()
      },
      "quality.maxFrameRate",
    ),
    (
      RenderQualityOptions {
        vegetation_detail_metres: Some(10_000.0),
        ..quality()
      },
      "quality.vegetationDetailMetres",
    ),
    (
      RenderQualityOptions {
        cloud_fade_metres: Some(f32::INFINITY),
        ..quality()
      },
      "quality.cloudFadeMetres",
    ),
  ];

  for (options, needle) in cases {
    rejected(engine().set_render_quality(options), &[needle]);
  }
}

#[test]
fn weather_rejects_bad_presets_and_times() {
  let weather = WeatherOptions::default;
  let long = "x".repeat(65);
  let many = (0..65)
    .map(|index| (format!("p{index}"), WeatherPreset::default()))
    .collect();
  let cases = [
    (
      WeatherOptions {
        state: WeatherKind::new(long.clone()),
        ..weather()
      },
      "at most 64 bytes",
    ),
    (
      WeatherOptions {
        presets: [(long, WeatherPreset::default())].into_iter().collect(),
        ..weather()
      },
      "at most 64 bytes",
    ),
    (
      WeatherOptions {
        presets: many,
        ..weather()
      },
      "at most 64 presets",
    ),
    (
      WeatherOptions {
        state_duration_seconds: 0.0,
        ..weather()
      },
      "weather.stateDurationSeconds",
    ),
    (
      WeatherOptions {
        transition_seconds: 1e6,
        ..weather()
      },
      "weather.transitionSeconds",
    ),
    (
      WeatherOptions {
        state: WeatherKind::new("sunny"),
        ..weather()
      },
      "is not a preset",
    ),
  ];

  for (options, needle) in cases {
    rejected(engine().set_weather(options), &[needle]);
  }

  for seconds in [f32::NAN, -1.0, 1e6] {
    rejected(engine().advance_weather(seconds), &["advanceWeather"]);
  }

  let time = TimeOfDayOptions {
    hours: 25.0,
    ..TimeOfDayOptions::default()
  };
  rejected(engine().set_time_of_day(time), &["timeOfDay.hours"]);
}

#[test]
fn fractal_generation_rejects_bad_options() {
  let options = FractalTerrainOptions::default;
  let mut noise = options().noise;
  noise.lacunarity = 9.0;
  let cases = [
    (
      FractalTerrainOptions {
        size: 48,
        ..options()
      },
      "power of two from 16 to 2048",
    ),
    (
      FractalTerrainOptions {
        size: 16_384,
        ..options()
      },
      "but it is 16384",
    ),
    (
      FractalTerrainOptions {
        size: 4096,
        ..options()
      },
      "power of two from 16 to 2048, but it is 4096. A larger terrain leaves too little of the 4 GiB of memory WebAssembly can address to rebuild and export it.",
    ),
    (
      FractalTerrainOptions {
        horizontal_scale_metres: 0.0,
        ..options()
      },
      "horizontalScaleMetres",
    ),
    (
      FractalTerrainOptions {
        vertical_scale: 1_000.0,
        ..options()
      },
      "verticalScale",
    ),
    (
      FractalTerrainOptions {
        shape: Some(vista_types::TerrainShapeOptions {
          island: Some(2.0),
          ..Default::default()
        }),
        ..options()
      },
      "shape.island",
    ),
    (
      FractalTerrainOptions { noise, ..options() },
      "noise.lacunarity",
    ),
  ];

  for (options, needle) in cases {
    rejected(
      futures_executor::block_on(engine().generate_fractal(options)),
      &[needle],
    );
  }
}

fn raw(width: u32, height: u32) -> RawHeightmapOptions {
  RawHeightmapOptions {
    width,
    height,
    sample_format: RawSampleFormat::Float32,
    byte_order: None,
    metres_per_sample: 10.0,
    height_scale_metres: 1.0,
    no_data_value: None,
    sea_level_metres: None,
    landform: None,
  }
}

#[test]
fn raw_heightmaps_reject_bad_sizes_and_scales() {
  let load = |bytes: &[u8], options: RawHeightmapOptions| {
    futures_executor::block_on(engine().load_raw_heightmap(bytes, options))
  };
  let bytes = vec![0; 16 * 16 * 4];

  // 2048 is set by WebAssembly's 4 GiB of memory (see MAX_TERRAIN_SIZE).
  for (width, height) in [
    (0, 4),
    (1, 4),
    (2049, 2),
    (4096, 4096),
    (u32::MAX, u32::MAX),
  ] {
    rejected(load(&bytes, raw(width, height)), &["from 2 to 2048"]);
  }

  rejected(
    load(&bytes, raw(4096, 4096)),
    &["but they are 4096 and 4096. A larger terrain leaves too little of the 4 GiB"],
  );

  rejected(
    load(&bytes[..10], raw(16, 16)),
    &["holds 10 bytes", "need 1024"],
  );

  for metres in [0.0, -1.0, 1e5, f32::NAN] {
    let options = RawHeightmapOptions {
      metres_per_sample: metres,
      ..raw(16, 16)
    };
    rejected(load(&bytes, options), &["metresPerSample", "10000"]);
  }

  for scale in BAD {
    let options = RawHeightmapOptions {
      height_scale_metres: scale,
      ..raw(16, 16)
    };
    rejected(load(&bytes, options), &["heightScaleMetres"]);
  }
}

#[test]
fn samples_that_cannot_be_ground_become_no_data() {
  let mut values = [5.0f32; 16];
  values[1] = f32::NAN;
  values[2] = f32::INFINITY;
  values[3] = -3.4e38;
  let bytes: Vec<u8> = values
    .iter()
    .flat_map(|value| value.to_le_bytes())
    .collect();
  let map = vista_wasm::dem::decode_raw_heightmap(&bytes, &raw(4, 4)).expect("valid size");

  assert_eq!(&map.no_data[..5], &[false, true, true, true, false]);
  assert!(map.heights.iter().all(|height| height.is_finite()));
  assert!(map.metadata.warnings[0].contains("3 samples"));
}

#[test]
fn geotiffs_reject_bad_options_and_headers() {
  let options = DemLoadOptions {
    vertical_scale: Some(f32::NAN),
    ..DemLoadOptions::default()
  };
  rejected(
    vista_wasm::dem::decode_geotiff(b"II*\0\x08\0\0\0", &options),
    &["verticalScale"],
  );
  rejected(
    vista_wasm::dem::decode_geotiff(b"II*\0\xff\xff\xff\xff", &DemLoadOptions::default()),
    &["outside the supplied buffer"],
  );
}

#[test]
fn painted_maps_reject_bad_shapes() {
  let mut engine = engine_with_terrain();

  for (width, height, len) in [(1, 4, 4), (8193, 2, 16), (4, 4, 15)] {
    let mask = WaterMask {
      width,
      height,
      data: vec![0; len],
    };
    rejected(engine.set_water_mask(Some(mask)), &["setWaterMask"]);
    let data = vec![0; len];
    rejected(
      engine.set_vegetation_mask(true, Some((width, height, &data))),
      &["setVegetationMasks"],
    );
    rejected(
      engine.set_biome_map(Some((width, height, &data)), 3),
      &["setBiomeMap"],
    );
  }

  let data = vec![200; 16];
  rejected(
    engine.set_biome_map(Some((4, 4, &data)), 3),
    &["biome indices run from 0 to"],
  );
  let unpainted = vec![255; 16];
  rejected(
    engine.set_biome_map(Some((4, 4, &unpainted)), 9),
    &["borderSamples must be from 0 to 8"],
  );
}

#[test]
fn custom_trees_reject_bad_arrays() {
  let mut engine = engine();
  let species = TreeSpeciesKind::ALL[0];
  let positions = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
  let normals = [0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0];
  let uvs = [0.0; 6];
  let mut set = |positions: &[f32], normals: &[f32], uvs: &[f32], indices: &[u32]| {
    engine.set_tree_model(species, positions, normals, uvs, indices, None, None)
  };

  assert!(set(&positions, &normals, &uvs, &[0, 1, 2]).is_ok());
  rejected(
    set(&positions, &normals, &uvs, &[0, 1, 3]),
    &["0 to 2", "one is 3"],
  );
  rejected(
    set(&positions, &normals, &uvs, &[0, 1]),
    &["three indices per triangle"],
  );
  rejected(
    set(&positions, &normals[..6], &uvs, &[0, 1, 2]),
    &["normals", "(9)"],
  );
  rejected(
    set(&positions, &normals, &uvs[..4], &[0, 1, 2]),
    &["uvs", "(6)"],
  );
  rejected(
    set(&positions, &normals, &uvs, &vec![0; 3 * 131_073]),
    &["393216"],
  );

  let mut far = positions;
  far[4] = 5_000.0;
  rejected(set(&far, &normals, &uvs, &[0, 1, 2]), &["within 1000 m"]);
  far[4] = f32::NAN;
  rejected(set(&far, &normals, &uvs, &[0, 1, 2]), &["within 1000 m"]);

  let tree = |values: [f32; 9]| engine_with_terrain().set_tree_instances(Some(unpacked(&values)));
  assert!(tree([0.0, 0.0, 0.0, 1.0, 0.0, 0.5, 0.0, 0.0, 1.0]).is_ok());

  for (index, value) in [
    (3, 0.0),
    (3, 21.0),
    (5, 1.5),
    (7, -0.5),
    (0, f32::NAN),
    (4, f32::INFINITY),
  ] {
    let mut values = [0.0, 0.0, 0.0, 1.0, 0.0, 0.5, 0.0, 0.0, 1.0];
    values[index] = value;
    rejected(tree(values), &["tree 0 must have"]);
  }

  for bad in [
    [0.0, 0.0, 0.0, 1.0, 0.0, 0.5, 9.0, 0.0, 1.0],
    [0.0, 0.0, 0.0, 1.0, 0.0, 0.5, 0.5, 0.0, 1.0],
  ] {
    assert!(vista_wasm::render::flora::unpack_tree_placements(&bad).is_err());
  }

  assert!(vista_wasm::render::flora::unpack_tree_placements(&[0.0; 10]).is_err());
}

fn unpacked(values: &[f32]) -> Vec<vista_wasm::render::flora::TreeInstance> {
  vista_wasm::render::flora::unpack_tree_placements(values).expect("well-formed tree")
}

#[test]
fn textures_and_exports_reject_bad_sizes() {
  let mut engine = engine_with_terrain();
  rejected(
    engine.replace_texture(TextureTarget::TerrainAlbedo, 999, &[0; 16]),
    &["texture layer must be between 0 and"],
  );
  rejected(
    engine.replace_texture(TextureTarget::TerrainAlbedo, 0, &[0; 16]),
    &["but 16 bytes were given"],
  );

  let kind = vista_wasm::export::MapKind::ALL[0];
  for size in [[1, 4], [4, 2049], [0, 0]] {
    rejected(engine.export_map(kind, Some(size)), &["from 2 to 2048"]);
  }

  rejected(
    engine.export_map(kind, Some([4096, 4096])),
    &["but it is [4096, 4096]. A larger terrain leaves too little of the 4 GiB"],
  );

  for region in [[f32::NAN, 0.0, 1.0, 1.0], [10.0, 0.0, 1.0, 1.0]] {
    rejected(
      engine.export_trees(Some(region), None),
      &["exportTrees region"],
    );
  }

  rejected(engine.export_trees(None, Some(0)), &["maxCount"]);
}

#[test]
fn queries_at_positions_that_are_not_finite_return_nothing() {
  let engine = engine_with_terrain();

  for value in BAD.into_iter().chain([1e30, -1e30]) {
    assert!(engine.biome_at(value, 0.0).is_none());
    assert!(engine.celsius_at(0.0, value).is_none());
    let sounds = engine.water_sounds(value, 0.0, value);
    assert!(sounds.river.is_none() || value.is_finite());
    let _ = engine.weather_at(value, value);
  }
}

/// A load or generation that fails part way leaves the engine as it was:
/// ready, with its terrain, and able to load again.
#[test]
fn a_failed_load_or_generation_leaves_the_engine_ready_and_usable() {
  let mut engine = engine_with_terrain();
  let before = engine.export_heightmap().expect("terrain");
  let failures: [VistaResult<vista_types::TerrainHandle>; 3] = [
    futures_executor::block_on(engine.generate_fractal(FractalTerrainOptions {
      size: 33,
      ..FractalTerrainOptions::default()
    })),
    futures_executor::block_on(engine.load_raw_heightmap(&[0u8; 10][..], raw(16, 16))),
    futures_executor::block_on(
      engine.load_dem_from_array_buffer(b"II*\0\x08\0\0\0", DemLoadOptions::default()),
    ),
  ];

  for failure in failures {
    assert!(failure.is_err());
  }

  assert_eq!(engine.state(), vista_types::EngineState::Ready);
  assert_eq!(engine.export_heightmap().expect("terrain"), before);
  let bytes = vec![0u8; 16 * 16 * 4];
  futures_executor::block_on(engine.load_raw_heightmap(&bytes, raw(16, 16)))
    .expect("the engine loads again");
  assert_eq!(engine.state(), vista_types::EngineState::Ready);
  assert!(engine.render_once().is_ok());
}

/// A few samples kilometres apart cover a vast area. Trees, grass and
/// boulders are placed over the whole map, so beyond 10,000 km² they are
/// left out with a warning, instead of the load taking minutes.
#[test]
fn a_vast_sparse_map_loads_promptly_without_vegetation() {
  let mut engine = engine();
  let (width, height) = (31, 5);
  let bytes: Vec<u8> = (0..width * height)
    .flat_map(|index| (100.0 + (index % 7) as f32 * 300.0).to_le_bytes())
    .collect();
  let options = RawHeightmapOptions {
    metres_per_sample: 10_000.0,
    ..raw(width, height)
  };
  let started = std::time::Instant::now();
  let handle =
    futures_executor::block_on(engine.load_raw_heightmap(&bytes, options)).expect("it loads");
  assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
  assert!(
    handle
      .metadata
      .warnings
      .iter()
      .any(|warning| warning.contains("more than the 10000 km²")),
    "{:?}",
    handle.metadata.warnings
  );
  let stats = engine.render_once().expect("renders");
  assert_eq!(stats.flora_instances, 0);
  // Reeds are grass, so they are left out too.
  assert_eq!(stats.grass_instances, 0);
}
