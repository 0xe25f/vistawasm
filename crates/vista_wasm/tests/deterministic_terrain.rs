use futures_executor::block_on;
use vista_types::{FractalTerrainOptions, NoiseKind, NoiseOptions, VistaEngineOptions};
use vista_wasm::engine::EngineCore;
use vista_wasm::terrain::generate_fractal_heightmap;

#[test]
fn fractal_generation_is_repeatable() {
  let options = FractalTerrainOptions {
    seed: 987_654,
    size: 32,
    horizontal_scale_metres: 8.0,
    vertical_scale: 1.2,
    noise: NoiseOptions {
      kind: NoiseKind::Hybrid,
      octaves: 5,
      gain: 0.5,
      lacunarity: 2.0,
      warp: Some(0.25),
    },
    ..FractalTerrainOptions::default()
  };
  let left = generate_fractal_heightmap(&options).unwrap();
  let right = generate_fractal_heightmap(&options).unwrap();

  assert_eq!(left.heights, right.heights);
}

#[test]
fn engine_can_generate_and_export_heightmap() {
  let mut engine = EngineCore::new_for_tests(VistaEngineOptions::default()).unwrap();
  let handle = block_on(engine.generate_fractal(FractalTerrainOptions {
    size: 32,
    ..FractalTerrainOptions::default()
  }))
  .unwrap();
  let bytes = engine.export_heightmap().unwrap();

  assert_eq!(handle.metadata.width, 32);
  assert_eq!(bytes.len(), 32 * 32 * 4);
}
