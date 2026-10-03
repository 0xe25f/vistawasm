use vista_types::{ByteOrder, RawHeightmapOptions, RawSampleFormat, TerrainMetadata};

use super::{sample_count, Samples};
use crate::config::{
  validate_finite, validate_height, validate_positive_at_most, MAX_METRES_PER_SAMPLE,
};
use crate::errors::{VistaError, VistaResult};
use crate::terrain::heightmap::HeightMap;

/// The largest buffer a raw heightmap load accepts: the largest map,
/// 2048 x 2048 float32 samples (16 MiB). A longer buffer can only be a mistake,
/// and refusing it keeps the copy into WASM memory bounded.
pub const MAX_RAW_HEIGHTMAP_BYTES: u64 =
  crate::config::MAX_TERRAIN_SIZE as u64 * crate::config::MAX_TERRAIN_SIZE as u64 * 4;

/// Bytes converted at a time: a raw load reads its input in pieces this
/// size, so the input is never copied into WASM memory whole.
const PIECE_BYTES: usize = 1 << 20;

/// Raw heightmap bytes, read a piece at a time: a slice natively, a view
/// of the host's `ArrayBuffer` in the browser.
pub trait RawBytes {
  /// The length in bytes.
  fn byte_length(&self) -> u64;
  /// Copy `out.len()` bytes from `offset`, which the caller keeps within
  /// the length.
  fn read(&self, offset: usize, out: &mut [u8]);
}

impl<T: AsRef<[u8]> + ?Sized> RawBytes for T {
  fn byte_length(&self) -> u64 {
    self.as_ref().len() as u64
  }

  fn read(&self, offset: usize, out: &mut [u8]) {
    out.copy_from_slice(&self.as_ref()[offset..offset + out.len()]);
  }
}

/// Decode raw heightmap bytes supplied by the host. Only the bytes the
/// samples need are read, a piece at a time; a longer buffer adds a
/// warning to the metadata, and one longer than any map could need
/// ([`MAX_RAW_HEIGHTMAP_BYTES`]) is rejected.
pub fn decode_raw_heightmap<B: RawBytes + ?Sized>(
  bytes: &B,
  options: &RawHeightmapOptions,
) -> VistaResult<HeightMap> {
  let length = bytes.byte_length();
  let sample_count = validate_options(length, options)?;
  let byte_order = options.byte_order.unwrap_or_default();
  let sample_size = sample_size(options.sample_format);
  let mut samples = Samples::new(
    sample_count,
    options.height_scale_metres,
    options.no_data_value,
  );
  let needed = sample_count * sample_size;
  let mut piece = vec![0u8; PIECE_BYTES.min(needed)];
  let mut offset = 0;

  // `validate_options` checked the buffer holds every sample, and a piece
  // is a whole number of samples.
  while offset < needed {
    let chunk = &mut piece[..PIECE_BYTES.min(needed - offset)];
    bytes.read(offset, chunk);
    offset += chunk.len();

    for sample in chunk.chunks_exact(sample_size) {
      samples.push(match options.sample_format {
        RawSampleFormat::Uint16 => f32::from(u16::from_be_bytes(two(sample, byte_order))),
        RawSampleFormat::Int16 => f32::from(i16::from_be_bytes(two(sample, byte_order))),
        RawSampleFormat::Float32 => f32::from_be_bytes(four(sample, byte_order)),
      });
    }
  }

  let mut warnings: Vec<String> = samples.warning().into_iter().collect();

  if length > needed as u64 {
    warnings.push(format!(
      "The raw heightmap buffer holds {length} bytes, but {} x {} samples of {sample_size} bytes need only {needed}; the rest was ignored. Pass a buffer of exactly {needed} bytes: a longer one becomes an error in the next major version.",
      options.width, options.height
    ));
  }

  let metadata = TerrainMetadata {
    width: options.width,
    height: options.height,
    metres_per_sample: options.metres_per_sample,
    vertical_scale: options.height_scale_metres,
    sea_level_metres: options.sea_level_metres.unwrap_or(0.0),
    source: "raw-heightmap".to_string(),
    generator_version: "vistawasm-raw-heightmap-0.1.0".to_string(),
    warnings,
    ..TerrainMetadata::default()
  };

  HeightMap::from_values(
    options.width,
    options.height,
    samples.heights,
    samples.no_data,
    metadata,
  )
}

/// Check the options against the buffer's length before anything is
/// allocated, and return the number of samples.
fn validate_options(length: u64, options: &RawHeightmapOptions) -> VistaResult<usize> {
  let count =
    sample_count("raw heightmap", options.width, options.height).map_err(VistaError::options)?;
  validate_positive_at_most(
    "raw heightmap metresPerSample",
    options.metres_per_sample,
    MAX_METRES_PER_SAMPLE,
  )?;
  validate_finite(
    "raw heightmap heightScaleMetres",
    options.height_scale_metres,
  )?;

  if let Some(sea) = options.sea_level_metres {
    validate_height("raw heightmap seaLevelMetres", sea)?;
  }

  let expected = count as u64 * sample_size(options.sample_format) as u64;

  if length < expected {
    return Err(VistaError::options(format!(
      "raw heightmap buffer holds {length} bytes, but {} x {} samples of {} bytes need {expected}.",
      options.width,
      options.height,
      sample_size(options.sample_format)
    )));
  }

  if length > MAX_RAW_HEIGHTMAP_BYTES {
    return Err(VistaError::options(format!(
      "raw heightmap buffer holds {length} bytes, more than the {MAX_RAW_HEIGHTMAP_BYTES} the largest map ({max} x {max} float32 samples) needs; {} x {} samples need {expected}.",
      options.width,
      options.height,
      max = crate::config::MAX_TERRAIN_SIZE
    )));
  }

  Ok(count)
}

fn sample_size(format: RawSampleFormat) -> usize {
  match format {
    RawSampleFormat::Uint16 | RawSampleFormat::Int16 => 2,
    RawSampleFormat::Float32 => 4,
  }
}

/// A two-byte sample in big-endian order, whatever order it was stored in.
fn two(chunk: &[u8], byte_order: ByteOrder) -> [u8; 2] {
  let bytes = [chunk[0], chunk[1]];

  match byte_order {
    ByteOrder::LittleEndian => [bytes[1], bytes[0]],
    ByteOrder::BigEndian => bytes,
  }
}

/// A four-byte sample in big-endian order, whatever order it was stored in.
fn four(chunk: &[u8], byte_order: ByteOrder) -> [u8; 4] {
  let bytes = [chunk[0], chunk[1], chunk[2], chunk[3]];

  match byte_order {
    ByteOrder::LittleEndian => [bytes[3], bytes[2], bytes[1], bytes[0]],
    ByteOrder::BigEndian => bytes,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn decodes_float32_raw_heightmap() {
    let bytes: Vec<u8> = [1.0_f32, 2.0, 3.0, 4.0]
      .iter()
      .flat_map(|value| value.to_le_bytes())
      .collect();
    let map = decode_raw_heightmap(
      &bytes,
      &RawHeightmapOptions {
        width: 2,
        height: 2,
        sample_format: RawSampleFormat::Float32,
        byte_order: Some(ByteOrder::LittleEndian),
        metres_per_sample: 30.0,
        height_scale_metres: 1.0,
        no_data_value: None,
        sea_level_metres: None,
        landform: None,
      },
    )
    .unwrap();

    assert_eq!(map.heights, vec![1.0, 2.0, 3.0, 4.0]);
  }

  fn options(width: u32, height: u32) -> RawHeightmapOptions {
    RawHeightmapOptions {
      width,
      height,
      sample_format: RawSampleFormat::Int16,
      byte_order: Some(ByteOrder::BigEndian),
      metres_per_sample: 30.0,
      height_scale_metres: 1.0,
      no_data_value: None,
      sea_level_metres: None,
      landform: None,
    }
  }

  /// Bytes that claim a length but hold nothing past their first few, as
  /// a huge host buffer would: only the bytes the samples need are read.
  struct Claimed(u64);

  impl RawBytes for Claimed {
    fn byte_length(&self) -> u64 {
      self.0
    }

    fn read(&self, offset: usize, out: &mut [u8]) {
      assert!(offset + out.len() <= 32, "read {offset} + {}", out.len());
      out.fill(0);
    }
  }

  #[test]
  fn a_longer_buffer_warns_and_only_its_samples_are_read() {
    let map = decode_raw_heightmap(&Claimed(1 << 20), &options(4, 4)).unwrap();
    assert_eq!(map.heights.len(), 16);
    let warning = map.metadata.warnings.join(" ");
    assert!(warning.contains("holds 1048576 bytes"), "{warning}");
    assert!(warning.contains("need only 32"), "{warning}");
  }

  #[test]
  fn a_buffer_past_the_largest_map_is_rejected_before_reading() {
    let error = decode_raw_heightmap(&Claimed(1 << 31), &options(4, 4)).unwrap_err();
    assert!(error.to_string().contains("2147483648 bytes"), "{error}");
  }

  #[test]
  fn large_maps_are_read_in_pieces_with_the_same_result() {
    // Over two pieces of int16 samples, with a different value in each
    // sample, so a piece read at the wrong offset would show.
    let (width, height) = (1024, 1100);
    let bytes: Vec<u8> = (0..width * height)
      .flat_map(|index| ((index % 30_000) as i16).to_be_bytes())
      .collect();
    let map = decode_raw_heightmap(&bytes, &options(width, height)).unwrap();

    for (index, height) in map.heights.iter().enumerate() {
      assert_eq!(*height, (index % 30_000) as f32);
    }

    assert!(map.metadata.warnings.is_empty());
  }
}
