use vista_types::{ByteOrder, RawHeightmapOptions, RawSampleFormat, TerrainMetadata};

use crate::errors::{VistaError, VistaResult};
use crate::terrain::heightmap::HeightMap;

/// Decode raw heightmap bytes supplied by the host.
pub fn decode_raw_heightmap(bytes: &[u8], options: &RawHeightmapOptions) -> VistaResult<HeightMap> {
  validate_options(bytes, options)?;

  let byte_order = options.byte_order.unwrap_or_default();
  let sample_count = options.width as usize * options.height as usize;
  let mut heights = Vec::with_capacity(sample_count);
  let mut no_data = Vec::with_capacity(sample_count);
  let sample_size = sample_size(options.sample_format);

  for index in 0..sample_count {
    let offset = index * sample_size;
    let value = match options.sample_format {
      RawSampleFormat::Uint16 => read_u16(bytes, offset, byte_order) as f32,
      RawSampleFormat::Int16 => read_i16(bytes, offset, byte_order) as f32,
      RawSampleFormat::Float32 => read_f32(bytes, offset, byte_order),
    };
    let scaled = value * options.height_scale_metres;
    let is_no_data = options
      .no_data_value
      .is_some_and(|marker| (value - marker).abs() <= f32::EPSILON);

    heights.push(if is_no_data { 0.0 } else { scaled });
    no_data.push(is_no_data);
  }

  let metadata = TerrainMetadata {
    width: options.width,
    height: options.height,
    metres_per_sample: options.metres_per_sample,
    vertical_scale: options.height_scale_metres,
    sea_level_metres: options.sea_level_metres.unwrap_or(0.0),
    source: "raw-heightmap".to_string(),
    generator_version: "vistawasm-raw-heightmap-0.1.0".to_string(),
    ..TerrainMetadata::default()
  };

  HeightMap::from_values(options.width, options.height, heights, no_data, metadata)
}

fn validate_options(bytes: &[u8], options: &RawHeightmapOptions) -> VistaResult<()> {
  if options.width == 0 || options.height == 0 {
    return Err(VistaError::options(
      "raw heightmap width and height must be at least 1.",
    ));
  }

  if !options.metres_per_sample.is_finite() || options.metres_per_sample <= 0.0 {
    return Err(VistaError::options(
      "raw heightmap metresPerSample must be greater than 0.",
    ));
  }

  if !options.height_scale_metres.is_finite() {
    return Err(VistaError::options(
      "raw heightmap heightScaleMetres must be finite.",
    ));
  }

  let expected =
    options.width as usize * options.height as usize * sample_size(options.sample_format);

  if bytes.len() < expected {
    return Err(VistaError::options(
      "raw heightmap buffer is smaller than width, height, and sample format require.",
    ));
  }

  Ok(())
}

fn sample_size(format: RawSampleFormat) -> usize {
  match format {
    RawSampleFormat::Uint16 | RawSampleFormat::Int16 => 2,
    RawSampleFormat::Float32 => 4,
  }
}

fn read_u16(bytes: &[u8], offset: usize, byte_order: ByteOrder) -> u16 {
  let array = [bytes[offset], bytes[offset + 1]];

  match byte_order {
    ByteOrder::LittleEndian => u16::from_le_bytes(array),
    ByteOrder::BigEndian => u16::from_be_bytes(array),
  }
}

fn read_i16(bytes: &[u8], offset: usize, byte_order: ByteOrder) -> i16 {
  let array = [bytes[offset], bytes[offset + 1]];

  match byte_order {
    ByteOrder::LittleEndian => i16::from_le_bytes(array),
    ByteOrder::BigEndian => i16::from_be_bytes(array),
  }
}

fn read_f32(bytes: &[u8], offset: usize, byte_order: ByteOrder) -> f32 {
  let array = [
    bytes[offset],
    bytes[offset + 1],
    bytes[offset + 2],
    bytes[offset + 3],
  ];

  match byte_order {
    ByteOrder::LittleEndian => f32::from_le_bytes(array),
    ByteOrder::BigEndian => f32::from_be_bytes(array),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn decodes_float32_raw_heightmap() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&1.0_f32.to_le_bytes());
    bytes.extend_from_slice(&2.0_f32.to_le_bytes());
    let map = decode_raw_heightmap(
      &bytes,
      &RawHeightmapOptions {
        width: 2,
        height: 1,
        sample_format: RawSampleFormat::Float32,
        byte_order: Some(ByteOrder::LittleEndian),
        metres_per_sample: 30.0,
        height_scale_metres: 1.0,
        no_data_value: None,
        sea_level_metres: None,
      },
    )
    .unwrap();

    assert_eq!(map.heights, vec![1.0, 2.0]);
  }
}
