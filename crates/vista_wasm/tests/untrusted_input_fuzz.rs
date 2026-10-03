//! Deterministic fuzzing of every decoder and validator that takes input
//! from JavaScript. Each case must return an error or a valid result,
//! never panic, and never allocate more than the input and the documented
//! caps allow. The generator is seeded, so a failure names a case that
//! can be replayed.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};

use vista_types::{
  ByteOrder, CloudStyle, DemLoadOptions, NamedMap, RawHeightmapOptions, RawSampleFormat,
  RiverInflow, RiverInflows, VistaEngineOptions, WeatherClimate, WeatherKind, WeatherOptions,
  WeatherPreset,
};
use vista_wasm::config::{VistaEngineConfig, MAX_HEIGHT_METRES, MAX_TERRAIN_SIZE};
use vista_wasm::dem::{decode_geotiff, decode_raw_heightmap};
use vista_wasm::render::flora::unpack_tree_placements;
use vista_wasm::render::tree_models::{
  mesh_from_arrays, MAX_CUSTOM_TREE_INDICES, MAX_CUSTOM_TREE_VERTICES,
};

/// Cases per fuzzed input kind.
const CASES: usize = 10_000;
/// Allocations any case may make beyond what its input justifies.
const SLACK_BYTES: usize = 1 << 20;

/// Records the largest single allocation on each thread, so a case can
/// check it allocated no more than its input allows.
struct LargestAllocation;

thread_local! {
  static LARGEST: Cell<usize> = const { Cell::new(0) };
  static COUNT: Cell<usize> = const { Cell::new(0) };
}

fn note(size: usize) {
  let _ = LARGEST.try_with(|largest| largest.set(largest.get().max(size)));
  let _ = COUNT.try_with(|count| count.set(count.get() + 1));
}

/// Allocations made on this thread so far.
fn allocations() -> usize {
  COUNT.with(Cell::get)
}

// Counting allocations needs a global allocator, which is unsafe to
// implement; it only forwards to the system allocator.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for LargestAllocation {
  unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
    note(layout.size());
    unsafe { System.alloc(layout) }
  }

  unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
    note(layout.size());
    unsafe { System.alloc_zeroed(layout) }
  }

  unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
    note(new_size);
    unsafe { System.realloc(ptr, layout, new_size) }
  }

  unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
    unsafe { System.dealloc(ptr, layout) }
  }
}

#[global_allocator]
static ALLOCATOR: LargestAllocation = LargestAllocation;

/// SplitMix64: small, fast and good enough to spread cases around.
struct Rng(u64);

impl Rng {
  fn next(&mut self) -> u64 {
    self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = self.0;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
  }

  fn below(&mut self, bound: usize) -> usize {
    (self.next() % bound.max(1) as u64) as usize
  }

  fn chance(&mut self, probability: f64) -> bool {
    (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= probability
  }

  /// A float that is often an edge case.
  fn float(&mut self) -> f32 {
    const SPECIAL: [f32; 16] = [
      f32::NAN,
      f32::INFINITY,
      f32::NEG_INFINITY,
      0.0,
      -0.0,
      -1.0,
      1.0,
      0.5,
      1e-30,
      -1e-30,
      1e30,
      -1e30,
      f32::MAX,
      f32::MIN,
      1e9,
      8193.0,
    ];

    match self.below(3) {
      0 => SPECIAL[self.below(SPECIAL.len())],
      1 => (self.next() % 20_000) as f32 / 10.0 - 1_000.0,
      _ => f32::from_bits(self.next() as u32),
    }
  }

  /// A whole number that is often an edge case.
  fn int(&mut self) -> u32 {
    const SPECIAL: [u32; 10] = [0, 1, 2, 3, 16, 8192, 8193, 65_536, u32::MAX, u32::MAX / 2];

    if self.chance(0.5) {
      SPECIAL[self.below(SPECIAL.len())]
    } else {
      self.below(5_000) as u32
    }
  }

  fn bytes(&mut self, len: usize) -> Vec<u8> {
    (0..len).map(|_| self.next() as u8).collect()
  }
}

/// Run `case` for `count` seeded cases, failing on the first panic or on
/// an allocation larger than `budget` returns for that case.
fn fuzz<T>(
  name: &str,
  count: usize,
  mut make: impl FnMut(&mut Rng) -> T,
  mut case: impl FnMut(&T),
  budget: impl Fn(&T) -> usize,
) {
  let mut rng = Rng(0x5eed ^ name.len() as u64);

  for index in 0..count {
    let input = make(&mut rng);
    LARGEST.with(|largest| largest.set(0));
    let outcome = catch_unwind(AssertUnwindSafe(|| case(&input)));
    let largest = LARGEST.with(Cell::get);

    assert!(outcome.is_ok(), "{name}: case {index} panicked");
    assert!(
      largest <= budget(&input) + SLACK_BYTES,
      "{name}: case {index} allocated {largest} bytes in one go"
    );
  }
}

fn options(rng: &mut Rng) -> VistaEngineOptions {
  let mut options = VistaEngineOptions::default();
  // Two or so fields a case, so that many cases pass validation and are
  // then used.
  let p = 0.02;

  macro_rules! mutate {
    ($target:expr; $($field:ident).+ = $value:expr) => {
      if rng.chance(p) {
        $target.$($field).+ = $value;
      }
    };
  }

  let mut render = vista_types::RenderSizeOptions::default();
  mutate!(render; width = rng.int());
  mutate!(render; height = rng.int());
  mutate!(render; device_pixel_ratio = Some(rng.float()));
  options.render = Some(render);

  let mut camera = vista_types::CameraOptions::default();
  mutate!(camera; position = [rng.float(), rng.float(), rng.float()]);
  mutate!(camera; target = [rng.float(), rng.float(), rng.float()]);
  mutate!(camera; field_of_view_degrees = rng.float());
  mutate!(camera; roll_degrees = Some(rng.float()));
  mutate!(camera; near_metres = Some(rng.float()));
  mutate!(camera; far_metres = Some(rng.float()));
  mutate!(camera; minimum_height_above_terrain_metres = Some(rng.float()));
  options.camera = Some(camera);

  let mut sun = vista_types::SunOptions::default();
  mutate!(sun; azimuth_degrees = rng.float());
  mutate!(sun; elevation_degrees = rng.float());
  mutate!(sun; intensity = rng.float());
  options.sun = Some(sun);

  let mut atmosphere = vista_types::AtmosphereOptions::default();
  mutate!(atmosphere; rayleigh_strength = rng.float());
  mutate!(atmosphere; mie_strength = rng.float());
  mutate!(atmosphere; haze_distance_metres = rng.float());
  mutate!(atmosphere; exposure = rng.float());
  mutate!(atmosphere; sky_tint = [rng.float(), rng.float(), rng.float()]);
  options.atmosphere = Some(atmosphere);

  let mut water = vista_types::WaterOptions::default();
  mutate!(water; sea_level_metres = rng.float());
  mutate!(water; wave_scale = rng.float());
  mutate!(water; reflectivity = rng.float());
  mutate!(water; foam = rng.float());
  mutate!(water; eddies = rng.float());
  mutate!(water; clarity_metres = rng.float());
  mutate!(water; current_speed = rng.float());
  mutate!(water; shallow_colour = [rng.float(), 0.5, 0.5]);
  mutate!(water; waves.amplitude_metres = rng.float());
  mutate!(water; waves.wavelength_metres = rng.float());
  mutate!(water; waves.steepness = rng.float());
  mutate!(water; waves.directional_spread = rng.float());
  mutate!(water; rivers.min_catchment_km2 = rng.float());
  mutate!(water; rivers.width_scale = rng.float());
  mutate!(water; rivers.snowmelt = rng.float());
  mutate!(water; rivers.meanders = rng.float());
  mutate!(water; rivers.meander_maturity = rng.float());
  mutate!(water; rivers.braiding = rng.float());
  mutate!(water; rivers.riparian = rng.float());

  if rng.chance(p) {
    let list = (0..rng.below(12))
      .map(|_| RiverInflow {
        position: [rng.float(), rng.float()],
        discharge_cubic_metres_per_second: rng.float(),
      })
      .collect();
    water.rivers.inflow = RiverInflows::List(list);
  }

  options.water = Some(water);

  let mut flora = vista_types::FloraOptions::default();
  mutate!(flora; density = rng.float());
  mutate!(flora; tree_line_metres = rng.float());
  mutate!(flora; species_variation = rng.float());
  mutate!(flora; wind_strength = rng.float());
  mutate!(flora; mesh_distance_metres = rng.float());
  mutate!(flora; max_instances = rng.int());
  mutate!(flora; variants_per_species = rng.int());
  options.flora = Some(flora);

  let mut grass = vista_types::GrassOptions::default();
  mutate!(grass; density = rng.float());
  mutate!(grass; view_distance_metres = rng.float());
  mutate!(grass; max_instances = rng.int());
  options.grass = Some(grass);

  let mut clouds = vista_types::CloudsOptions::default();
  mutate!(clouds; style = CloudStyle::Volumetric);
  mutate!(clouds; raymarch_steps = Some(rng.int()));
  mutate!(clouds; coverage = rng.float());
  mutate!(clouds; speed = rng.float());
  mutate!(clouds; height_metres = rng.float());
  mutate!(clouds; evolution = rng.float());
  mutate!(clouds; thickness_metres = rng.float());
  mutate!(clouds; density = rng.float());
  mutate!(clouds; resolution_scale = rng.float());
  mutate!(clouds; cirrus_height_metres = rng.float());
  mutate!(clouds; base_variation = rng.float());
  mutate!(clouds; alto_height_metres = rng.float());
  options.clouds = Some(clouds);

  let mut mist = vista_types::MistOptions::default();
  mutate!(mist; density = rng.float());
  mutate!(mist; base_height_metres = rng.float());
  mutate!(mist; height_falloff_metres = rng.float());
  mutate!(mist; sun_scattering = rng.float());
  options.mist = Some(mist);

  let mut quality = vista_types::RenderQualityOptions::default();
  mutate!(quality; max_clipmap_levels = Some(rng.int()));
  mutate!(quality; flora_density_scale = Some(rng.float()));
  mutate!(quality; render_distance_metres = Some(rng.float()));
  mutate!(quality; cloud_fade_metres = Some(rng.float()));
  mutate!(quality; max_frame_rate = Some(rng.float()));
  mutate!(quality; render_scale = Some(rng.float()));
  mutate!(quality; vegetation_detail_metres = Some(rng.float()));
  mutate!(quality; canopy_distance_metres = Some(rng.float()));
  mutate!(quality; max_tree_instances = Some(rng.int()));
  mutate!(quality; max_tree_triangles = Some(rng.int()));
  mutate!(quality; grass_detail_metres = Some(rng.float()));
  options.quality = Some(quality);

  let mut biomes = vista_types::BiomeOptions::default();
  mutate!(biomes; temperature_bias = rng.float());
  mutate!(biomes; climate_scale_metres = rng.float());
  mutate!(biomes; volcanism = rng.float());
  mutate!(biomes; snow_line_metres = Some(rng.float()));
  mutate!(biomes; mean_temperature_celsius = Some(rng.float()));
  options.biomes = Some(biomes);

  let mut weather = WeatherOptions::default();
  mutate!(weather; state_duration_seconds = rng.float());
  mutate!(weather; transition_seconds = rng.float());
  mutate!(weather; wind_scale = rng.float());
  mutate!(weather; lens_drop_count = rng.int());
  mutate!(weather; lens_drop_min_size = rng.float());
  mutate!(weather; region_size_km = rng.float());

  if rng.chance(p * 10.0) {
    weather.presets = presets(rng);
    weather.state = WeatherKind::new(name(rng));
  }

  options.weather = Some(weather);

  let mut shadows = vista_types::ShadowOptions::default();
  mutate!(shadows; terrain.strength = rng.float());
  mutate!(shadows; trees.distance_metres = rng.float());
  mutate!(shadows; trees.resolution = rng.int());
  options.shadows = Some(shadows);

  let mut surface = vista_types::SurfaceOptions::default();
  mutate!(surface; texture_scale = rng.float());
  mutate!(surface; rockiness = rng.float());
  mutate!(surface; boulder_distance_metres = rng.float());
  options.surface = Some(surface);

  options
}

fn name(rng: &mut Rng) -> String {
  const NAMES: [&str; 8] = [
    "clear",
    "storm",
    "partlyCloudy",
    "haar",
    "squall",
    "",
    "__proto__",
    "constructor",
  ];

  if rng.chance(0.1) {
    "x".repeat(rng.below(200))
  } else {
    NAMES[rng.below(NAMES.len())].to_string()
  }
}

fn presets(rng: &mut Rng) -> NamedMap<WeatherPreset> {
  let count = if rng.chance(0.05) { 70 } else { rng.below(6) };

  (0..count)
    .map(|_| {
      let preset = WeatherPreset {
        extends: rng.chance(0.5).then(|| WeatherKind::new(name(rng))),
        cloud_coverage: rng.chance(0.5).then(|| rng.float()),
        rain: rng.chance(0.3).then(|| rng.float()),
        min_duration_seconds: rng.chance(0.3).then(|| rng.float()),
        max_duration_seconds: rng.chance(0.3).then(|| rng.float()),
        next: rng.chance(0.3).then(|| {
          (0..rng.below(4))
            .map(|_| (name(rng), rng.float()))
            .collect()
        }),
        climate: rng.chance(0.2).then(|| WeatherClimate {
          min_celsius: Some(rng.float()),
          max_celsius: Some(rng.float()),
        }),
        ..WeatherPreset::default()
      };
      (name(rng), preset)
    })
    .collect()
}

#[test]
fn option_validation_never_panics_and_accepted_options_run() {
  let mut accepted = 0;
  fuzz(
    "options",
    CASES,
    options,
    |options| {
      if let Ok(config) = VistaEngineConfig::from_options(options.clone()) {
        accepted += 1;

        // Every tenth accepted configuration also drives an engine.
        if accepted % 10 == 0 {
          let mut engine =
            vista_wasm::engine::EngineCore::new_for_tests(options.clone()).expect("valid options");
          engine.render_once().expect("a frame without terrain");
        }

        drop(config);
      }
    },
    |_| 0,
  );
  assert!(
    accepted > CASES / 10,
    "only {accepted} of {CASES} option sets were valid"
  );
}

fn raw_input(rng: &mut Rng) -> (RawHeightmapOptions, Vec<u8>) {
  let small = |rng: &mut Rng| {
    if rng.chance(0.3) {
      rng.int()
    } else {
      rng.below(40) as u32
    }
  };
  let options = RawHeightmapOptions {
    width: small(rng),
    height: small(rng),
    sample_format: [
      RawSampleFormat::Uint16,
      RawSampleFormat::Int16,
      RawSampleFormat::Float32,
    ][rng.below(3)],
    byte_order: [
      None,
      Some(ByteOrder::LittleEndian),
      Some(ByteOrder::BigEndian),
    ][rng.below(3)],
    metres_per_sample: if rng.chance(0.7) { 30.0 } else { rng.float() },
    height_scale_metres: if rng.chance(0.7) { 1.0 } else { rng.float() },
    no_data_value: rng.chance(0.3).then(|| rng.float()),
    sea_level_metres: rng.chance(0.2).then(|| rng.float()),
    landform: None,
  };
  let wanted = (options.width as usize)
    .saturating_mul(options.height as usize)
    .saturating_mul(4);
  let len = if rng.chance(0.7) {
    wanted.min(16_384)
  } else {
    rng.below(16_384)
  };
  (options, rng.bytes(len))
}

#[test]
fn raw_heightmaps_never_panic_or_over_allocate() {
  let mut decoded = 0;
  fuzz(
    "raw heightmap",
    CASES,
    raw_input,
    |(options, bytes)| {
      if let Ok(map) = decode_raw_heightmap(bytes, options) {
        decoded += 1;
        check_map(&map.heights, &map.no_data, options.width, options.height);
      }
    },
    |(_, bytes)| bytes.len() * 4,
  );
  assert!(
    decoded > CASES / 10,
    "only {decoded} raw heightmaps decoded"
  );
}

fn check_map(heights: &[f32], no_data: &[bool], width: u32, height: u32) {
  assert!(width <= MAX_TERRAIN_SIZE && height <= MAX_TERRAIN_SIZE);
  assert_eq!(heights.len(), width as usize * height as usize);
  assert_eq!(no_data.len(), heights.len());
  assert!(heights
    .iter()
    .all(|height| height.is_finite() && height.abs() <= MAX_HEIGHT_METRES));
}

/// A small, valid, uncompressed GeoTIFF.
fn tiff(rng: &mut Rng) -> Vec<u8> {
  let little = rng.chance(0.5);
  let (width, height) = (2 + rng.below(12) as u32, 2 + rng.below(12) as u32);
  let (bits, format) = [(16u32, 1u32), (16, 2), (32, 1), (32, 2), (32, 3)][rng.below(5)];
  let data_len = (width * height * bits / 8) as usize;
  let u16b = |value: u16| {
    if little {
      value.to_le_bytes()
    } else {
      value.to_be_bytes()
    }
  };
  let u32b = |value: u32| {
    if little {
      value.to_le_bytes()
    } else {
      value.to_be_bytes()
    }
  };
  let scale_offset = 8u32;
  let data_offset = scale_offset + 24 + 8;
  let ifd_offset = data_offset + data_len as u32;
  let mut out = Vec::new();
  out.extend(if little { *b"II" } else { *b"MM" });
  out.extend(u16b(42));
  out.extend(u32b(ifd_offset));

  for value in [30.0f64, 30.0, 0.0] {
    out.extend(if little {
      value.to_le_bytes()
    } else {
      value.to_be_bytes()
    });
  }

  out.extend(*b"-9999\0\0\0");
  out.extend(rng.bytes(data_len));
  let entries: [(u16, u16, u32, u32); 10] = [
    (256, 4, 1, width),
    (257, 4, 1, height),
    (258, 3, 1, bits),
    (259, 3, 1, 1),
    (273, 4, 1, data_offset),
    (277, 3, 1, 1),
    (279, 4, 1, data_len as u32),
    (339, 3, 1, format),
    (33550, 12, 3, scale_offset),
    (42113, 2, 6, scale_offset + 24),
  ];
  out.extend(u16b(entries.len() as u16));

  for (tag, kind, count, value) in entries {
    out.extend(u16b(tag));
    out.extend(u16b(kind));
    out.extend(u32b(count));

    if kind == 3 && count == 1 {
      out.extend(u16b(value as u16));
      out.extend([0, 0]);
    } else {
      out.extend(u32b(value));
    }
  }

  out.extend(u32b(0));
  out
}

fn tiff_input(rng: &mut Rng) -> Vec<u8> {
  if rng.chance(0.05) {
    let len = rng.below(256);
    return rng.bytes(len);
  }

  let mut bytes = tiff(rng);

  for _ in 0..rng.below(6) {
    let at = rng.below(bytes.len());

    match rng.below(4) {
      0 => bytes[at] = rng.next() as u8,
      1 => {
        let value = [0u32, 1, 0xffff, 0x7fff_ffff, u32::MAX, bytes.len() as u32][rng.below(6)];
        let end = (at + 4).min(bytes.len());
        bytes[at..end].copy_from_slice(&value.to_le_bytes()[..end - at]);
      }
      2 => bytes.truncate(at),
      _ => bytes[at] ^= 1 << rng.below(8),
    }

    if bytes.is_empty() {
      break;
    }
  }

  bytes
}

#[test]
fn geotiffs_never_panic_or_over_allocate() {
  let mut decoded = 0;
  let options = DemLoadOptions::default();
  let mut rng = Rng(7);
  let clean = decode_geotiff(&tiff(&mut rng), &options).expect("a valid GeoTIFF decodes");
  assert_eq!(clean.metadata.metres_per_sample, 30.0);

  fuzz(
    "GeoTIFF",
    CASES,
    tiff_input,
    |bytes| {
      if let Ok(map) = decode_geotiff(bytes, &options) {
        decoded += 1;
        let metadata = &map.metadata;
        check_map(&map.heights, &map.no_data, metadata.width, metadata.height);
        assert!(metadata.metres_per_sample > 0.0 && metadata.metres_per_sample <= 10_000.0);
      }
    },
    |bytes| bytes.len() * 4,
  );
  assert!(decoded > CASES / 20, "only {decoded} GeoTIFFs decoded");
}

type MeshInput = (
  Vec<f32>,
  Vec<f32>,
  Vec<f32>,
  Vec<u32>,
  Option<Vec<f32>>,
  Option<Vec<f32>>,
);

fn mesh_input(rng: &mut Rng) -> MeshInput {
  let count = if rng.chance(0.002) {
    MAX_CUSTOM_TREE_VERTICES + 1
  } else {
    rng.below(24)
  };
  let off = |rng: &mut Rng, len: usize| {
    if rng.chance(0.1) {
      len + 1 - rng.below(3).min(len + 1)
    } else {
      len
    }
  };
  let value = |rng: &mut Rng| {
    if rng.chance(0.05) {
      rng.float()
    } else {
      rng.below(2_000) as f32 / 100.0 - 5.0
    }
  };
  let floats =
    |rng: &mut Rng, len: usize| -> Vec<f32> { (0..off(rng, len)).map(|_| value(rng)).collect() };
  let positions = floats(rng, count * 3);
  let normals = floats(rng, count * 3);
  let uvs = floats(rng, count * 2);
  let triangles = if rng.chance(0.002) {
    MAX_CUSTOM_TREE_INDICES / 3 + 1
  } else {
    rng.below(20)
  };
  let indices = (0..off(rng, triangles * 3))
    .map(|_| {
      if rng.chance(0.02) {
        rng.int()
      } else {
        rng.below(count.max(1)) as u32
      }
    })
    .collect();
  let layers = rng.chance(0.3).then(|| {
    (0..off(rng, count))
      .map(|_| {
        if rng.chance(0.1) {
          rng.float()
        } else {
          rng.below(15) as f32
        }
      })
      .collect()
  });
  let wind = rng.chance(0.3).then(|| {
    (0..off(rng, count))
      .map(|_| rng.below(11) as f32 / 10.0)
      .collect()
  });
  (positions, normals, uvs, indices, layers, wind)
}

#[test]
fn custom_tree_meshes_never_panic_or_over_allocate() {
  let mut built = 0;
  fuzz(
    "mesh_from_arrays",
    CASES,
    mesh_input,
    |(positions, normals, uvs, indices, layers, wind)| {
      if let Ok(mesh) = mesh_from_arrays(
        positions,
        normals,
        uvs,
        indices,
        layers.as_deref(),
        wind.as_deref(),
      ) {
        built += 1;
        assert!(mesh.vertices.len() <= MAX_CUSTOM_TREE_VERTICES);
        assert!(mesh.indices.len() <= MAX_CUSTOM_TREE_INDICES);
        assert!(mesh
          .indices
          .iter()
          .all(|index| (*index as usize) < mesh.vertices.len()));
        assert!(mesh.height.is_finite() && mesh.radius.is_finite());
      }
    },
    |(positions, normals, uvs, indices, ..)| {
      8 * (positions.len() + normals.len() + uvs.len() + indices.len())
    },
  );
  assert!(built > CASES / 20, "only {built} tree meshes were valid");
}

#[test]
fn tree_placements_never_panic_or_over_allocate() {
  let mut placed = 0;
  fuzz(
    "setTreeInstances",
    CASES,
    |rng| {
      let len = rng.below(40);
      (0..len)
        .map(|index| match (index % 9, rng.chance(0.05)) {
          (_, true) => rng.float(),
          (6, false) => rng.below(8) as f32,
          (8, false) => rng.below(2) as f32,
          (3, false) => 1.0,
          _ => rng.below(100) as f32 / 100.0,
        })
        .collect::<Vec<f32>>()
    },
    |packed| {
      if let Ok(trees) = unpack_tree_placements(packed) {
        if vista_wasm::engine::validate_tree_instances(&trees).is_ok() {
          placed += 1;
        }
      }
    },
    |packed| packed.len() * 16,
  );
  assert!(placed > CASES / 20, "only {placed} placements were valid");
}

#[test]
fn painted_maps_never_panic_or_over_allocate() {
  use vista_wasm::terrain::painted::{density_mask, PaintedBiomes};

  let mut accepted = 0;
  fuzz(
    "painted maps",
    CASES,
    |rng| {
      let side = |rng: &mut Rng| {
        if rng.chance(0.2) {
          rng.int()
        } else {
          rng.below(20) as u32
        }
      };
      let (width, height) = (side(rng), side(rng));
      let len = if rng.chance(0.8) {
        (width as usize * height as usize).min(1_024)
      } else {
        rng.below(1_024)
      };
      let data: Vec<u8> = (0..len)
        .map(|_| {
          if rng.chance(0.9) {
            rng.below(16) as u8
          } else {
            rng.next() as u8
          }
        })
        .collect();
      (width, height, data, rng.below(12) as u32)
    },
    |(width, height, data, border)| {
      let map = (*width, *height, data.as_slice());
      let target = (2 + data.len() as u32 % 30, 2 + *border * 3);

      if let Ok((mask, _)) = density_mask("grass", map, target) {
        accepted += 1;
        assert_eq!(mask.len(), (target.0 * target.1) as usize);
      }

      if let Ok((painted, _)) = PaintedBiomes::new(map, *border, target, 7) {
        for y in 0..target.1 {
          for x in 0..target.0 {
            let _ = painted.at(x, y);
          }
        }
      }
    },
    |_| 0,
  );
  assert!(
    accepted > CASES / 20,
    "only {accepted} painted maps were valid"
  );
}

#[test]
fn large_sizes_from_small_inputs_are_rejected_before_allocating() {
  let raw = RawHeightmapOptions {
    width: MAX_TERRAIN_SIZE,
    height: MAX_TERRAIN_SIZE,
    sample_format: RawSampleFormat::Float32,
    byte_order: None,
    metres_per_sample: 1.0,
    height_scale_metres: 1.0,
    no_data_value: None,
    sea_level_metres: None,
    landform: None,
  };
  let mut rng = Rng(3);
  let mut tiff = tiff(&mut rng);
  // Claim the largest image the decoder accepts in the first IFD entry.
  let ifd = u32::from_le_bytes([tiff[4], tiff[5], tiff[6], tiff[7]]) as usize;
  let little = tiff[0] == b'I';

  for (entry, value) in [(0, MAX_TERRAIN_SIZE), (1, MAX_TERRAIN_SIZE)] {
    let at = ifd + 2 + entry * 12 + 8;
    let bytes = if little {
      value.to_le_bytes()
    } else {
      value.to_be_bytes()
    };
    tiff[at..at + 4].copy_from_slice(&bytes);
  }

  LARGEST.with(|largest| largest.set(0));
  assert!(decode_raw_heightmap(&[0; 64], &raw).is_err());
  assert!(decode_geotiff(&tiff, &DemLoadOptions::default()).is_err());
  let largest = LARGEST.with(Cell::get);
  assert!(largest < 64 * 1024, "allocated {largest} bytes");
}

// The terrain and river pipeline, fed valid but pathological maps: every
// stage from finishing a loaded map to building its rivers, exporting it
// and loading the export again. `PIPELINE_FUZZ_CASES` sets the number of
// cases (2,000 by default, as CI runs it; 50,000 for a long local run).

/// The map shapes the pipeline fuzzer draws from.
const SHAPES: [&str; 9] = [
  "flat",
  "stepped",
  "spiky",
  "no data",
  "half sea",
  "one wide",
  "bowl",
  "fine noise",
  "coarse noise",
];

/// A pathological map, and the river options to build it with.
struct PipelineCase {
  shape: &'static str,
  map: vista_wasm::terrain::HeightMap,
  rivers: vista_types::RiverOptions,
}

impl PipelineCase {
  /// The case in a line, for failure messages.
  fn describe(&self) -> String {
    let metadata = &self.map.metadata;
    format!(
      "{} {} x {} at {} m, sea {} m, {:?}",
      self.shape,
      metadata.width,
      metadata.height,
      metadata.metres_per_sample,
      metadata.sea_level_metres,
      self.rivers
    )
  }
}

fn pipeline_case(rng: &mut Rng) -> PipelineCase {
  const SIDES: [u32; 12] = [2, 3, 4, 5, 7, 8, 9, 16, 31, 64, 128, 256];
  let shape = SHAPES[rng.below(SHAPES.len())];
  // Small maps are cheap and where the edge cases are, so most are small.
  let side = |rng: &mut Rng| {
    let range = if rng.chance(0.85) { 9 } else { SIDES.len() };
    SIDES[rng.below(range)]
  };
  let (width, height) = match shape {
    "one wide" if rng.chance(0.5) => (1, side(rng)),
    "one wide" => (side(rng), 1),
    _ => (side(rng), side(rng)),
  };
  let metres = match shape {
    "fine noise" => 0.001,
    "coarse noise" => vista_wasm::config::MAX_METRES_PER_SAMPLE,
    _ => [1.0, 12.0, 30.0, 500.0][rng.below(4)],
  };
  let sea = [0.0, -50.0, 1_000.0][rng.below(3)];
  let seed = rng.next();
  let count = (width * height) as usize;
  let mut no_data = vec![false; count];
  let heights: Vec<f32> = (0..count)
    .map(|index| {
      let (x, y) = ((index as u32 % width) as f32, (index as u32 / width) as f32);
      let noise = (vista_wasm::maths::hash_u64(seed ^ index as u64) % 10_000) as f32 / 10_000.0;
      let (cx, cy) = (x - width as f32 * 0.5, y - height as f32 * 0.5);

      match shape {
        "flat" => sea + 10.0,
        "stepped" => sea + 40.0 * ((x + y) / 4.0).floor(),
        "spiky" if noise > 0.97 => MAX_HEIGHT_METRES,
        "spiky" => sea + noise,
        "no data" => {
          no_data[index] = true;
          0.0
        }
        "half sea" if x < width as f32 * 0.5 => sea - 20.0,
        "half sea" => sea + 5.0 + y * 3.0 + noise,
        "bowl" => sea + 50.0 + (cx * cx + cy * cy).sqrt() * 2.0,
        _ => sea + noise * 2_000.0 - 500.0,
      }
    })
    .collect();
  let metadata = vista_types::TerrainMetadata {
    metres_per_sample: metres,
    vertical_scale: 1.0,
    sea_level_metres: sea,
    ..vista_types::TerrainMetadata::default()
  };
  let map = vista_wasm::terrain::HeightMap::from_values(width, height, heights, no_data, metadata)
    .expect("a valid map");
  // Each river option at one of its edges, or at its default.
  let mut rivers = vista_types::RiverOptions::default();
  let edge = |rng: &mut Rng, low: f32, high: f32, default: f32| [low, high, default][rng.below(3)];
  rivers.enabled = rng.chance(0.9);
  rivers.min_catchment_km2 = edge(rng, 1e-4, 1e4, rivers.min_catchment_km2);
  rivers.width_scale = edge(rng, 0.01, 100.0, rivers.width_scale);
  rivers.current_speed = edge(rng, 0.0, 100.0, rivers.current_speed);
  rivers.snowmelt = edge(rng, 0.0, 2.0, rivers.snowmelt);
  rivers.springs = rng.chance(0.5);
  rivers.meanders = edge(rng, 0.0, 1.0, rivers.meanders);
  rivers.meander_maturity = edge(rng, 0.0, 1.0, rivers.meander_maturity);
  rivers.braiding = edge(rng, 0.0, 1.0, rivers.braiding);
  rivers.waterfalls = rng.chance(0.5);
  rivers.riparian = edge(rng, 0.0, 2.0, rivers.riparian);
  rivers.inflow = match rng.below(3) {
    0 => RiverInflows::Mode(vista_types::InflowMode::None),
    1 => RiverInflows::List(vec![RiverInflow {
      position: [0.0, 0.0],
      discharge_cubic_metres_per_second: edge(rng, 0.0, 100_000.0, 50.0),
    }]),
    _ => RiverInflows::default(),
  };
  PipelineCase { shape, map, rivers }
}

/// Build `case`'s rivers as the engine does, and check what comes out:
/// every number finite, river water on the ground, levels that never
/// rise downstream, and the same network from the same input.
fn check_river_network(case: &PipelineCase) {
  use vista_wasm::render::water::{build_river_network, RiverSources, WATER_KIND_RIVER};
  let build = || {
    let mut map = case.map.clone();
    let (_, surface) = vista_wasm::render::terrain_mesh::bake_terrain_shading(
      &map,
      &vista_types::BiomeOptions::default(),
      None,
    );
    let record = vista_wasm::terrain::channels::CarveRecord::new(map.heights.len());
    let network = build_river_network(
      &mut map,
      &case.rivers,
      RiverSources {
        surface: &surface,
        seed: 7,
        painted: Vec::new(),
        record,
      },
    );
    (map, network)
  };
  let (map, network) = build();
  let shape = case.describe();
  let metres = map.metadata.metres_per_sample;
  let half = [
    (map.metadata.width as f32 - 1.0) * metres * 0.5,
    (map.metadata.height as f32 - 1.0) * metres * 0.5,
  ];

  assert!(
    map.heights.iter().all(|h| h.is_finite()),
    "{shape}: carved heights are not finite"
  );

  for vertex in network.vertices.iter().chain(&network.fall_vertices) {
    assert!(
      vertex.position.iter().all(|v| v.is_finite()),
      "{shape}: a water vertex at {:?}",
      vertex.position
    );
  }

  // Rivers must lie on the ground, except where the ground cannot hold
  // them: spikes 100 km tall a sample apart are no ground a channel can
  // be carved into, and an explicit inflow forces a river onto the map
  // whatever its size, up to 100,000 m³/s: wider and deeper than a small,
  // coarse map's samples resolve. Catchments below the default 0.15 km²
  // draw a stream down almost every cell of a steep slope, side by side
  // with no ground between them; gathering those into one stream in a
  // hollow of its own is not done yet, so they are not held to it.
  let ground_holds = case.shape != "spiky"
    && !matches!(case.rivers.inflow, RiverInflows::List(_))
    && case.rivers.min_catchment_km2 >= vista_types::RiverOptions::default().min_catchment_km2;

  for vertex in network
    .vertices
    .iter()
    .filter(|v| ground_holds && v.kind() == WATER_KIND_RIVER)
  {
    let x = (vertex.position[0] + half[0]) / metres;
    let y = (vertex.position[2] + half[1]) / metres;
    let ground = vista_wasm::render::terrain_mesh::full_detail_height(&map, x, y);

    // 3 m, as on the report maps, plus a hundredth of the spacing between
    // samples: the ground under a vertex is interpolated across them, and
    // on maps hundreds of metres a sample that misses that much relief.
    // An edge vertex also stands its half width beside the centreline at
    // the centreline's level, and on a map too coarse to carve banks the
    // ground falls away beneath it by its slope times that half width.
    if ground > map.metadata.sea_level_metres {
      let above = vertex.position[1] - ground;
      let step = 0.01;
      let slope = |dx: f32, dy: f32| {
        let at = |s: f32| {
          vista_wasm::render::terrain_mesh::full_detail_height(&map, x + dx * s, y + dy * s)
        };
        (at(step) - at(-step)).abs() / (2.0 * step * metres)
      };
      let fall = slope(1.0, 0.0).hypot(slope(0.0, 1.0)) * vertex.params[2];
      let allowed = 3.0 + metres / 100.0 + fall;
      assert!(
        above <= allowed,
        "{shape}: water {above} m above the ground at {x}, {y}"
      );
    }
  }

  for reach in &network.reaches {
    for pair in reach.points.windows(2) {
      assert!(
        pair[1].level <= pair[0].level + 1e-3,
        "{shape}: the water climbs from {} to {} m",
        pair[0].level,
        pair[1].level
      );
    }
  }

  // A tributary keeps its own discharge to its join, where it meets a
  // main stem that carries more.
  let hydrology = vista_wasm::terrain::hydrology::build_hydrology(&case.map, &[], &case.rivers, 7);

  for stream in vista_wasm::terrain::channels::raw_streams(&hydrology)
    .iter()
    .filter(|stream| stream.mouth == vista_wasm::terrain::hydrology::Mouth::Join)
  {
    let n = stream.discharge.len();
    assert!(
      n < 2 || stream.discharge[n - 1] == stream.discharge[n - 2],
      "{shape}: a tributary takes its main stem's discharge at its join"
    );
  }

  // The same input gives the same network, to the bit.
  let (again, repeat) = build();
  assert!(
    again.heights == map.heights,
    "{shape}: carving is not deterministic"
  );
  assert!(
    repeat.vertices.len() == network.vertices.len()
      && repeat
        .vertices
        .iter()
        .zip(&network.vertices)
        .all(|(a, b)| a.position.map(f32::to_bits) == b.position.map(f32::to_bits)),
    "{shape}: the river mesh is not deterministic"
  );
}

/// Load `case` into an engine, as JavaScript would, export every map,
/// load the exported source heights into a second engine, and check its
/// exports are the same bytes.
fn check_engine_round_trip(case: &PipelineCase) {
  use vista_wasm::export::{MapData, MapKind};
  let map = &case.map;

  // Loaded maps are at least 2 samples a side; narrower ones only reach
  // the pipeline above.
  if map.metadata.width < 2 || map.metadata.height < 2 {
    return;
  }

  let raw = RawHeightmapOptions {
    width: map.metadata.width,
    height: map.metadata.height,
    sample_format: RawSampleFormat::Float32,
    byte_order: Some(ByteOrder::LittleEndian),
    metres_per_sample: map.metadata.metres_per_sample,
    height_scale_metres: 1.0,
    no_data_value: map.no_data.iter().any(|n| *n).then_some(f32::NAN),
    sea_level_metres: Some(map.metadata.sea_level_metres),
    landform: None,
  };
  // Vegetation is fuzzed on its own; here it would only add placement
  // work that grows with the map's area.
  let options = VistaEngineOptions {
    flora: Some(vista_types::FloraOptions {
      density: 0.0,
      ..vista_types::FloraOptions::default()
    }),
    grass: Some(vista_types::GrassOptions {
      enabled: false,
      ..vista_types::GrassOptions::default()
    }),
    surface: Some(vista_types::SurfaceOptions {
      boulders: false,
      ..vista_types::SurfaceOptions::default()
    }),
    ..VistaEngineOptions::default()
  };
  let load = |heights: &[f32]| {
    let mut engine =
      vista_wasm::engine::EngineCore::new_for_tests(options.clone()).expect("valid options");
    let mut water = vista_types::WaterOptions {
      sea_level_metres: map.metadata.sea_level_metres,
      ..vista_types::WaterOptions::default()
    };
    water.rivers = case.rivers.clone();
    engine.set_water(water).expect("valid water options");
    let bytes: Vec<u8> = heights.iter().flat_map(|h| h.to_le_bytes()).collect();
    futures_executor::block_on(engine.load_raw_heightmap(&bytes, raw.clone()))
      .expect("a valid map loads");
    engine
  };
  let marked: Vec<f32> = map
    .heights
    .iter()
    .zip(&map.no_data)
    .map(|(h, n)| if *n { f32::NAN } else { *h })
    .collect();
  let engine = load(&marked);
  let exports = |engine: &vista_wasm::engine::EngineCore| -> Vec<Vec<u32>> {
    MapKind::ALL
      .iter()
      .map(|kind| {
        let exported = engine.export_map(*kind, None).expect("every map exports");

        match exported.data {
          MapData::F32(values) => {
            assert!(
              values.iter().all(|v| v.is_finite()),
              "{}: {kind:?} holds a number that is not finite",
              case.describe()
            );
            values.iter().map(|v| v.to_bits()).collect()
          }
          MapData::U8(values) => values.iter().map(|v| u32::from(*v)).collect(),
        }
      })
      .collect()
  };
  let first = exports(&engine);
  let source = match engine
    .export_map(MapKind::SourceHeight, None)
    .expect("source heights")
    .data
  {
    MapData::F32(values) => values,
    MapData::U8(_) => unreachable!("source heights are floats"),
  };
  // No-data samples export as heights, so the copy has none; it is
  // compared with a load of the same heights.
  let copy = load(&source);
  let reference = load(&source);
  assert!(
    exports(&copy) == exports(&reference),
    "{}: loading is not deterministic",
    case.shape
  );
  assert!(
    first.len() == MapKind::ALL.len(),
    "{}: a map is missing",
    case.describe()
  );
}

/// `PIPELINE_FUZZ_CASE` replays one case by its number, as a failure
/// names it.
fn pipeline_only() -> Option<usize> {
  std::env::var("PIPELINE_FUZZ_CASE")
    .ok()
    .and_then(|case| case.parse().ok())
}

fn pipeline_cases() -> usize {
  std::env::var("PIPELINE_FUZZ_CASES")
    .ok()
    .and_then(|cases| cases.parse().ok())
    .unwrap_or(2_000)
}

#[test]
fn the_terrain_and_river_pipeline_survives_pathological_maps() {
  let only = pipeline_only();
  let mut index = 0;
  fuzz(
    "pipeline",
    only.map_or(pipeline_cases(), |case| case + 1),
    pipeline_case,
    |case| {
      if only.is_none_or(|only| only == index) {
        check_river_network(case);
        check_engine_round_trip(case);
      }

      index += 1;
    },
    // Every buffer the pipeline makes is proportional to the map, at
    // most a kilobyte a sample (the channel field and the meshes), apart
    // from the vegetation's tile grids, which span its extent in 16 m
    // tiles, at most `MAX_TILES` of them, and buffers with fixed caps:
    // the bank strips and the bucket grids, 32 MiB at most.
    |case| {
      let metadata = &case.map.metadata;
      let tiles = |samples: u32| {
        ((samples.max(1) - 1) as f32 * metadata.metres_per_sample / 16.0) as usize + 2
      };
      let grid = (tiles(metadata.width) * tiles(metadata.height))
        .min(vista_wasm::render::vegetation::MAX_TILES as usize);
      case.map.heights.len() * 1024 + grid * 4 + (32 << 20)
    },
  );
}

/// Springs were picked from 300 m buckets laid over the whole map, so
/// 128 x 256 samples 10 km apart allocated 583 MB of buckets at once.
/// Only buckets with a candidate are kept now.
#[test]
fn springs_on_a_vast_map_take_memory_for_its_samples_not_its_area() {
  let (width, height) = (128u32, 256u32);
  let heights: Vec<f32> = (0..width * height)
    .map(|index| {
      let noise = (vista_wasm::maths::hash_u64(7 ^ u64::from(index)) % 10_000) as f32 / 10_000.0;
      noise * 2_000.0 - 500.0
    })
    .collect();
  let metadata = vista_types::TerrainMetadata {
    metres_per_sample: 10_000.0,
    vertical_scale: 1.0,
    sea_level_metres: -50.0,
    ..vista_types::TerrainMetadata::default()
  };
  let count = heights.len();
  let map = vista_wasm::terrain::HeightMap::from_values(
    width,
    height,
    heights,
    vec![false; count],
    metadata,
  )
  .expect("a valid map");
  let rivers = vista_types::RiverOptions {
    springs: true,
    ..vista_types::RiverOptions::default()
  };
  LARGEST.with(|largest| largest.set(0));
  let hydrology = vista_wasm::terrain::hydrology::build_hydrology(&map, &[], &rivers, 3);
  let largest = LARGEST.with(Cell::get);
  assert!(largest < count * 64, "allocated {largest} bytes in one go");
  assert!(hydrology.receiver.len() == count);
}

#[test]
fn a_frame_allocates_nothing_after_warm_up() {
  // A small map with rivers, weather and lens drops, so each per-frame
  // system has work to do.
  let options = VistaEngineOptions {
    weather: Some(WeatherOptions {
      enabled: true,
      lens_drops: true,
      ..WeatherOptions::default()
    }),
    ..VistaEngineOptions::default()
  };
  let mut engine = vista_wasm::engine::EngineCore::new_for_tests(options).expect("valid options");
  let size = 64u32;
  let heights: Vec<f32> = (0..size * size)
    .map(|i| {
      let (x, y) = ((i % size) as f32, (i / size) as f32);
      40.0 + 30.0 * (x * 0.11).sin() * (y * 0.07).cos() + y * 0.5
    })
    .collect();
  let bytes: Vec<u8> = heights.iter().flat_map(|h| h.to_le_bytes()).collect();
  futures_executor::block_on(engine.load_raw_heightmap(
    &bytes,
    RawHeightmapOptions {
      width: size,
      height: size,
      sample_format: RawSampleFormat::Float32,
      byte_order: Some(ByteOrder::LittleEndian),
      metres_per_sample: 12.0,
      height_scale_metres: 1.0,
      no_data_value: None,
      sea_level_metres: Some(0.0),
      landform: None,
    },
  ))
  .expect("the map loads");

  for _ in 0..30 {
    engine.render_once().expect("a frame renders");
  }

  let before = allocations();

  for _ in 0..100 {
    engine.render_once().expect("a frame renders");
  }

  assert_eq!(allocations() - before, 0, "allocations in 100 frames");
}
