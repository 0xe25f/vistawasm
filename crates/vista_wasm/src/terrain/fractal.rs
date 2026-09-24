use vista_types::{FractalTerrainOptions, NoiseKind, TerrainMetadata};

use crate::errors::{VistaError, VistaResult};
use crate::maths::{clamp_f32, hash_u64, lerp};
use crate::terrain::erosion::apply_erosion;
use crate::terrain::heightmap::HeightMap;
use crate::terrain::noise::{noise_seed, simplex};

const GENERATOR_VERSION: &str = "vistawasm-fractal-0.1.0";

/// Generate a deterministic CPU reference heightmap, then apply CPU
/// reference erosion if requested.
///
/// Browser builds prefer GPU compute erosion for performance (see
/// `render::erosion_compute`) and call
/// [`generate_fractal_heightmap_base`] directly instead. This function
/// remains the single-call path used by native tests and as a fallback.
pub fn generate_fractal_heightmap(options: &FractalTerrainOptions) -> VistaResult<HeightMap> {
  let mut map = generate_fractal_heightmap_base(options)?;

  if let Some(erosion) = &options.erosion {
    apply_erosion(&mut map, erosion)?;
  }

  Ok(map)
}

/// Generate a deterministic CPU reference heightmap without applying
/// erosion, so a caller can apply erosion separately (for example, on the
/// GPU).
pub fn generate_fractal_heightmap_base(options: &FractalTerrainOptions) -> VistaResult<HeightMap> {
  validate_fractal_options(options)?;

  let size = options.size;
  let len = size as usize * size as usize;
  let mut heights = vec![0.0; len];
  let no_data = vec![false; len];
  let warp = options.noise.warp.unwrap_or(0.0).clamp(0.0, 4.0);
  let base_height = options.base_height_metres.unwrap_or(0.0);

  for y in 0..size {
    for x in 0..size {
      let nx = x as f32 / (size - 1) as f32;
      let ny = y as f32 / (size - 1) as f32;
      let warped = warp_domain(options.seed, nx, ny, warp);
      let mut value = octave_noise(options, warped.0, warped.1);
      value = shape_noise(options, nx, ny, value);

      let metres = base_height + value * 900.0 * options.vertical_scale;
      heights[(y * size + x) as usize] = metres;
    }
  }

  let metadata = TerrainMetadata {
    width: size,
    height: size,
    metres_per_sample: options.horizontal_scale_metres,
    vertical_scale: options.vertical_scale,
    sea_level_metres: options.sea_level_metres.unwrap_or(0.0),
    source: "fractal".to_string(),
    generator_version: GENERATOR_VERSION.to_string(),
    ..TerrainMetadata::default()
  };

  HeightMap::from_values(size, size, heights, no_data, metadata)
}

fn validate_fractal_options(options: &FractalTerrainOptions) -> VistaResult<()> {
  if options.size < 16 || options.size > 8192 || !options.size.is_power_of_two() {
    return Err(VistaError::options(
      "fractal size must be a power of two between 16 and 8192.",
    ));
  }

  if !options.horizontal_scale_metres.is_finite() || options.horizontal_scale_metres <= 0.0 {
    return Err(VistaError::options(
      "horizontalScaleMetres must be a finite value greater than 0.",
    ));
  }

  if !options.vertical_scale.is_finite() || options.vertical_scale <= 0.0 {
    return Err(VistaError::options(
      "verticalScale must be a finite value greater than 0.",
    ));
  }

  if options.noise.octaves == 0 || options.noise.octaves > 16 {
    return Err(VistaError::options(
      "noise.octaves must be between 1 and 16.",
    ));
  }

  if !options.noise.gain.is_finite() || !(0.0..=1.0).contains(&options.noise.gain) {
    return Err(VistaError::options("noise.gain must be between 0 and 1."));
  }

  if !options.noise.lacunarity.is_finite() || options.noise.lacunarity <= 1.0 {
    return Err(VistaError::options(
      "noise.lacunarity must be a finite value greater than 1.",
    ));
  }

  Ok(())
}

fn warp_domain(seed: u64, x: f32, y: f32, warp: f32) -> (f32, f32) {
  if warp <= 0.0 {
    return (x, y);
  }

  let dx = simplex(noise_seed(seed, 0xa53a), x * 2.0, y * 2.0) * warp * 0.08;
  let dy = simplex(noise_seed(seed, 0x5ac3), x * 2.0 + 19.0, y * 2.0 - 7.0) * warp * 0.08;

  (x + dx, y + dy)
}

fn octave_noise(options: &FractalTerrainOptions, x: f32, y: f32) -> f32 {
  let mut amplitude = 1.0;
  let mut frequency = 1.0;
  let mut sum = 0.0;
  let mut normaliser = 0.0;

  for octave in 0..options.noise.octaves {
    let seed = hash_u64(options.seed ^ octave as u64);
    let sample = simplex(
      noise_seed(seed, 0),
      x * frequency * 6.0,
      y * frequency * 6.0,
    );
    let shaped = match options.noise.kind {
      NoiseKind::Simplex => sample,
      NoiseKind::Ridged => 1.0 - sample.abs() * 2.0,
      NoiseKind::Hybrid => lerp(sample, 1.0 - sample.abs() * 2.0, 0.45),
      NoiseKind::Island => sample,
      NoiseKind::Canyon => canyon_sample(sample, x, y),
      NoiseKind::Cratered => crater_sample(seed, sample, x, y),
      NoiseKind::Classic => classic_sample(sample),
    };

    sum += shaped * amplitude;
    normaliser += amplitude;
    amplitude *= options.noise.gain;
    frequency *= options.noise.lacunarity;
  }

  if normaliser <= f32::EPSILON {
    return 0.0;
  }

  clamp_f32(sum / normaliser, -1.0, 1.0)
}

fn shape_noise(options: &FractalTerrainOptions, x: f32, y: f32, mut value: f32) -> f32 {
  let kind = options.noise.kind;

  if matches!(kind, NoiseKind::Island) {
    value = apply_island(x, y, value, 0.85);
  }

  if matches!(kind, NoiseKind::Canyon) {
    value -= canyon_mask(x, y) * 0.6;
  }

  if matches!(kind, NoiseKind::Cratered) {
    value -= crater_mask(options.seed, x, y) * 0.5;
  }

  if let Some(shape) = &options.shape {
    if let Some(amount) = shape.island {
      value = apply_island(x, y, value, amount);
    }

    if let Some(amount) = shape.basin {
      let centre = distance_from_centre(x, y);
      value -= (1.0 - centre).max(0.0) * amount.clamp(0.0, 1.0);
    }

    if let Some(amount) = shape.canyon {
      value -= canyon_mask(x, y) * amount.clamp(0.0, 1.0);
    }

    if let Some(amount) = shape.crater {
      value -= crater_mask(options.seed, x, y) * amount.clamp(0.0, 1.0);
    }

    if let Some(terrace) = shape.terrace {
      value = terrace_value(value, terrace);
    }
  }

  clamp_f32(value, -1.0, 1.0)
}

fn apply_island(x: f32, y: f32, value: f32, amount: f32) -> f32 {
  let falloff = distance_from_centre(x, y).powf(1.5).clamp(0.0, 1.0);
  value - falloff * amount.clamp(0.0, 1.2)
}

fn distance_from_centre(x: f32, y: f32) -> f32 {
  let dx = x * 2.0 - 1.0;
  let dy = y * 2.0 - 1.0;
  (dx * dx + dy * dy).sqrt().clamp(0.0, 1.0)
}

fn canyon_sample(sample: f32, x: f32, y: f32) -> f32 {
  sample - canyon_mask(x, y) * 0.35
}

fn canyon_mask(x: f32, y: f32) -> f32 {
  let channel = (x * 1.8 + (y * 8.0).sin() * 0.08 - 0.9).abs();
  (1.0 - channel * 5.0).clamp(0.0, 1.0)
}

fn crater_sample(seed: u64, sample: f32, x: f32, y: f32) -> f32 {
  sample - crater_mask(seed, x, y) * 0.45
}

fn crater_mask(seed: u64, x: f32, y: f32) -> f32 {
  let mut mask: f32 = 0.0;

  for index in 0..5 {
    let hx = (hash_u64(seed ^ (index * 19) as u64) % 10_000) as f32 / 10_000.0;
    let hy = (hash_u64(seed ^ (index * 31) as u64) % 10_000) as f32 / 10_000.0;
    let radius = 0.04 + (hash_u64(seed ^ (index * 43) as u64) % 800) as f32 / 10_000.0;
    let dx = x - hx;
    let dy = y - hy;
    let distance = (dx * dx + dy * dy).sqrt();
    let bowl = (1.0 - distance / radius).clamp(0.0, 1.0);
    let rim = (1.0 - ((distance - radius) / (radius * 0.35)).abs()).clamp(0.0, 1.0);
    mask = mask.max(bowl * 0.8 - rim * 0.25);
  }

  mask
}

fn classic_sample(sample: f32) -> f32 {
  let stepped = (sample * 9.0).round() / 9.0;
  lerp(sample, stepped, 0.25)
}

fn terrace_value(value: f32, terrace: f32) -> f32 {
  let amount = terrace.clamp(0.0, 1.0);

  if amount <= 0.0 {
    return value;
  }

  let steps = 12.0;
  let stepped = ((value + 1.0) * 0.5 * steps).round() / steps * 2.0 - 1.0;
  lerp(value, stepped, amount)
}

#[cfg(test)]
mod tests {
  use super::*;
  use vista_types::{NoiseKind, NoiseOptions};

  #[test]
  fn same_seed_produces_same_heights() {
    let options = FractalTerrainOptions {
      size: 32,
      noise: NoiseOptions {
        kind: NoiseKind::Ridged,
        octaves: 4,
        gain: 0.5,
        lacunarity: 2.0,
        warp: Some(0.2),
      },
      ..FractalTerrainOptions::default()
    };
    let left = generate_fractal_heightmap(&options).unwrap();
    let right = generate_fractal_heightmap(&options).unwrap();

    assert_eq!(left.heights, right.heights);
  }
}
