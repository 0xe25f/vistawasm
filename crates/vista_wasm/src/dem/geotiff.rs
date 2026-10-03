use std::collections::HashMap;

use vista_types::{DemLoadOptions, GeospatialMetadata, TerrainMetadata};

use super::{sample_count, Samples};
use crate::config::{validate_finite, MAX_METRES_PER_SAMPLE};
use crate::errors::{VistaError, VistaResult};
use crate::terrain::heightmap::HeightMap;

const TAG_IMAGE_WIDTH: u16 = 256;
const TAG_IMAGE_LENGTH: u16 = 257;
const TAG_BITS_PER_SAMPLE: u16 = 258;
const TAG_COMPRESSION: u16 = 259;
const TAG_STRIP_OFFSETS: u16 = 273;
const TAG_SAMPLES_PER_PIXEL: u16 = 277;
const TAG_STRIP_BYTE_COUNTS: u16 = 279;
const TAG_MODEL_PIXEL_SCALE: u16 = 33550;
const TAG_MODEL_TIEPOINT: u16 = 33922;
const TAG_SAMPLE_FORMAT: u16 = 339;
const TAG_GEO_KEY_DIRECTORY: u16 = 34735;
const TAG_GDAL_NODATA: u16 = 42113;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Endian {
  Little,
  Big,
}

#[derive(Clone, Debug)]
struct IfdEntry {
  field_type: u16,
  count: u32,
  value_offset: u32,
  inline_value: [u8; 4],
}

/// The largest GeoTIFF file accepted: the largest map, 2048 x 2048
/// float32 samples, and 64 MiB for its tags and any padding between
/// strips. The whole file is copied into WASM memory to be parsed, so
/// the browser wrapper and the engine check this first.
pub const MAX_GEOTIFF_BYTES: u64 =
  crate::config::MAX_TERRAIN_SIZE as u64 * crate::config::MAX_TERRAIN_SIZE as u64 * 4
    + 64 * 1024 * 1024;

/// Reject a GeoTIFF longer than [`MAX_GEOTIFF_BYTES`] before reading it.
pub fn check_geotiff_length(length: u64) -> VistaResult<()> {
  if length > MAX_GEOTIFF_BYTES {
    return Err(VistaError::DemFormatUnsupported(format!(
      "the GeoTIFF is {length} bytes, more than the {MAX_GEOTIFF_BYTES} a map of at most {max} x {max} samples needs.",
      max = crate::config::MAX_TERRAIN_SIZE
    )));
  }

  Ok(())
}

/// Decode an uncompressed GeoTIFF into a heightmap.
///
/// Every offset and count in the file is untrusted: reads are bounds
/// checked with overflow-free arithmetic, and the image size is checked
/// against [`crate::config::MAX_TERRAIN_SIZE`] before anything is
/// allocated for it.
pub fn decode_geotiff(bytes: &[u8], options: &DemLoadOptions) -> VistaResult<HeightMap> {
  check_geotiff_length(bytes.len() as u64)?;
  let vertical_scale = options.vertical_scale.unwrap_or(1.0);
  validate_finite("verticalScale", vertical_scale)?;
  let endian = parse_endian(bytes)?;
  let magic = read_u16(bytes, 2, endian)?;

  if magic != 42 {
    return Err(VistaError::DemFormatUnsupported(
      "only classic TIFF files with magic value 42 are supported.".to_string(),
    ));
  }

  let ifd_offset = read_u32(bytes, 4, endian)? as usize;
  let entries = read_ifd(bytes, ifd_offset, endian)?;
  let width = entry_u32(bytes, &entries, TAG_IMAGE_WIDTH, endian)?;
  let height = entry_u32(bytes, &entries, TAG_IMAGE_LENGTH, endian)?;
  let expected =
    sample_count("GeoTIFF", width, height).map_err(VistaError::DemFormatUnsupported)?;
  let compression = entry_u32(bytes, &entries, TAG_COMPRESSION, endian).unwrap_or(1);

  if compression != 1 {
    return Err(VistaError::DemFormatUnsupported(
      "compressed GeoTIFF data is not supported yet. Use uncompressed strips.".to_string(),
    ));
  }

  let samples_per_pixel = entry_u32(bytes, &entries, TAG_SAMPLES_PER_PIXEL, endian).unwrap_or(1);

  if samples_per_pixel != 1 {
    return Err(VistaError::DemFormatUnsupported(
      "GeoTIFF files with more than one sample per pixel are not supported yet.".to_string(),
    ));
  }

  let bits_per_sample = entry_u32(bytes, &entries, TAG_BITS_PER_SAMPLE, endian)?;
  let sample_format = entry_u32(bytes, &entries, TAG_SAMPLE_FORMAT, endian).unwrap_or(1);
  let read_sample: fn(&[u8], Endian) -> f32 = match (bits_per_sample, sample_format) {
    (16, 1) => |chunk, endian| f32::from(read_chunk_u16(chunk, endian)),
    (16, 2) => |chunk, endian| f32::from(read_chunk_u16(chunk, endian) as i16),
    (32, 1) => |chunk, endian| read_chunk_u32(chunk, endian) as f32,
    (32, 2) => |chunk, endian| read_chunk_u32(chunk, endian) as i32 as f32,
    (32, 3) => |chunk, endian| f32::from_bits(read_chunk_u32(chunk, endian)),
    _ => {
      return Err(VistaError::DemFormatUnsupported(
        "GeoTIFF samples must be uint16, int16, uint32, int32, or float32.".to_string(),
      ));
    }
  };
  let sample_size = bits_per_sample as usize / 8;
  let strip_offsets = entry_values_u32(bytes, &entries, TAG_STRIP_OFFSETS, endian)?;
  let strip_byte_counts = entry_values_u32(bytes, &entries, TAG_STRIP_BYTE_COUNTS, endian)?;

  if strip_offsets.len() != strip_byte_counts.len() {
    return Err(VistaError::DemMetadataMissing(
      "strip offset and byte count arrays have different lengths.".to_string(),
    ));
  }

  // Strips are separate parts of the file, so together they cannot be
  // longer than it; checking this first means a small file can never
  // make the decoder allocate for a large image.
  let strip_bytes: u64 = strip_byte_counts
    .iter()
    .map(|count| u64::from(*count))
    .sum();
  let needed = expected as u64 * sample_size as u64;

  if strip_bytes > bytes.len() as u64 || strip_bytes < needed {
    return Err(VistaError::DemMetadataMissing(format!(
      "GeoTIFF strips hold {strip_bytes} bytes, but {width} x {height} samples need {needed} and the file has {}.",
      bytes.len()
    )));
  }

  let no_data_value = entry_ascii(bytes, &entries, TAG_GDAL_NODATA)
    .and_then(|text| text.trim_matches(char::from(0)).trim().parse::<f32>().ok());
  let pixel_scale = entry_values_f64(bytes, &entries, TAG_MODEL_PIXEL_SCALE, endian)
    .ok()
    .and_then(|values| array3(values.as_slice()));
  let tiepoint = entry_values_f64(bytes, &entries, TAG_MODEL_TIEPOINT, endian)
    .ok()
    .and_then(|values| array6(values.as_slice()));
  let mut warnings = geo_warnings(&entries);
  let mut samples = Samples::new(expected, vertical_scale, no_data_value);

  for (offset, byte_count) in strip_offsets.iter().zip(strip_byte_counts.iter()) {
    let strip = slice(bytes, *offset as usize, *byte_count as usize).map_err(|_| {
      VistaError::DemMetadataMissing(
        "GeoTIFF strip points outside the supplied buffer.".to_string(),
      )
    })?;
    let room = expected - samples.heights.len();

    for chunk in strip.chunks_exact(sample_size).take(room) {
      samples.push(read_sample(chunk, endian));
    }
  }

  // Strips of a length that is not a whole number of samples can still
  // leave the image short.
  if samples.heights.len() < expected {
    return Err(VistaError::DemMetadataMissing(format!(
      "GeoTIFF holds {} samples, but its width and height ({width} x {height}) need {expected}.",
      samples.heights.len()
    )));
  }

  // Scales outside this range are usually degrees rather than metres, and
  // would make world coordinates that are not finite.
  let scale = pixel_scale.map(|scale| [scale[0] as f32, scale[1] as f32]);
  let metres = scale
    .map(|[x, _]| x)
    .filter(|metres| *metres > 0.0 && *metres <= MAX_METRES_PER_SAMPLE);

  if scale.is_some() && metres.is_none() {
    warnings.push(format!(
      "ModelPixelScaleTag gives {} per sample, outside 0 to {MAX_METRES_PER_SAMPLE} m; metres per sample defaults to 1.",
      scale.map_or(0.0, |[x, _]| x)
    ));
  }

  warnings.extend(samples.warning());
  let mut metadata = TerrainMetadata {
    width,
    height,
    metres_per_sample: metres.unwrap_or(1.0),
    vertical_scale,
    sea_level_metres: 0.0,
    source: "geotiff".to_string(),
    generator_version: "vistawasm-geotiff-0.1.0".to_string(),
    geospatial: Some(GeospatialMetadata {
      metres_per_sample_x: scale.map(|[x, _]| x),
      metres_per_sample_y: scale.map(|[_, y]| y),
      projection_name: None,
      model_tiepoint: tiepoint,
      model_pixel_scale: pixel_scale,
    }),
    warnings,
    ..TerrainMetadata::default()
  };

  if !entries.contains_key(&TAG_GEO_KEY_DIRECTORY) {
    metadata
      .warnings
      .push("GeoKeyDirectoryTag is missing; projection is approximate.".to_string());
  }

  HeightMap::from_values(width, height, samples.heights, samples.no_data, metadata)
}

/// `len` bytes from `start`, or an error when any of them lies outside
/// `bytes` (or the end does not fit in `usize`).
fn slice(bytes: &[u8], start: usize, len: usize) -> VistaResult<&[u8]> {
  start
    .checked_add(len)
    .and_then(|end| bytes.get(start..end))
    .ok_or_else(|| {
      VistaError::DemMetadataMissing(
        "a TIFF offset or count points outside the supplied buffer.".to_string(),
      )
    })
}

fn parse_endian(bytes: &[u8]) -> VistaResult<Endian> {
  if bytes.len() < 8 {
    return Err(VistaError::DemFormatUnsupported(
      "TIFF header is shorter than 8 bytes.".to_string(),
    ));
  }

  match &bytes[0..2] {
    b"II" => Ok(Endian::Little),
    b"MM" => Ok(Endian::Big),
    _ => Err(VistaError::DemFormatUnsupported(
      "TIFF byte order must be II or MM.".to_string(),
    )),
  }
}

fn read_ifd(bytes: &[u8], offset: usize, endian: Endian) -> VistaResult<HashMap<u16, IfdEntry>> {
  let count = read_u16(bytes, offset, endian)? as usize;
  // At most 65,535 twelve-byte entries, all inside the buffer.
  let table = slice(bytes, offset.saturating_add(2), count * 12)?;
  let mut entries = HashMap::new();

  for entry in table.chunks_exact(12) {
    let tag = read_chunk_u16(&entry[0..2], endian);
    let field_type = read_chunk_u16(&entry[2..4], endian);
    let count = read_chunk_u32(&entry[4..8], endian);
    let value_offset = read_chunk_u32(&entry[8..12], endian);
    let inline_value = [entry[8], entry[9], entry[10], entry[11]];

    entries.insert(
      tag,
      IfdEntry {
        field_type,
        count,
        value_offset,
        inline_value,
      },
    );
  }

  Ok(entries)
}

fn geo_warnings(entries: &HashMap<u16, IfdEntry>) -> Vec<String> {
  let mut warnings = Vec::new();

  if !entries.contains_key(&TAG_MODEL_PIXEL_SCALE) {
    warnings.push("ModelPixelScaleTag is missing; metres per sample defaults to 1.".to_string());
  }

  if !entries.contains_key(&TAG_MODEL_TIEPOINT) {
    warnings.push("ModelTiepointTag is missing; terrain origin is unknown.".to_string());
  }

  warnings
}

fn entry_u32(
  bytes: &[u8],
  entries: &HashMap<u16, IfdEntry>,
  tag: u16,
  endian: Endian,
) -> VistaResult<u32> {
  entry_values_u32(bytes, entries, tag, endian)?
    .first()
    .copied()
    .ok_or_else(|| VistaError::DemMetadataMissing(format!("TIFF tag {tag} has no value.")))
}

fn entry(entries: &HashMap<u16, IfdEntry>, tag: u16) -> VistaResult<&IfdEntry> {
  entries
    .get(&tag)
    .ok_or_else(|| VistaError::DemMetadataMissing(format!("required TIFF tag {tag} is missing.")))
}

fn entry_values_u32(
  bytes: &[u8],
  entries: &HashMap<u16, IfdEntry>,
  tag: u16,
  endian: Endian,
) -> VistaResult<Vec<u32>> {
  let entry = entry(entries, tag)?;
  let raw = entry_raw_value(bytes, entry)?;

  // `raw` holds exactly `count` values, so the vectors below grow no
  // larger than the file itself.
  match entry.field_type {
    3 => Ok(
      raw
        .chunks_exact(2)
        .map(|chunk| u32::from(read_chunk_u16(chunk, endian)))
        .collect(),
    ),
    4 => Ok(
      raw
        .chunks_exact(4)
        .map(|chunk| read_chunk_u32(chunk, endian))
        .collect(),
    ),
    _ => Err(VistaError::DemMetadataMissing(format!(
      "TIFF tag {tag} has an unsupported numeric type."
    ))),
  }
}

fn entry_values_f64(
  bytes: &[u8],
  entries: &HashMap<u16, IfdEntry>,
  tag: u16,
  endian: Endian,
) -> VistaResult<Vec<f64>> {
  let entry = entry(entries, tag)?;
  let raw = entry_raw_value(bytes, entry)?;
  let chunks = raw.chunks_exact(8);

  match entry.field_type {
    5 => Ok(
      chunks
        .map(|chunk| {
          let numerator = read_chunk_u32(&chunk[0..4], endian);
          let denominator = read_chunk_u32(&chunk[4..8], endian).max(1);
          f64::from(numerator) / f64::from(denominator)
        })
        .collect(),
    ),
    12 => Ok(
      chunks
        .map(|chunk| {
          let first = u64::from(read_chunk_u32(&chunk[0..4], endian));
          let second = u64::from(read_chunk_u32(&chunk[4..8], endian));
          f64::from_bits(match endian {
            Endian::Little => second << 32 | first,
            Endian::Big => first << 32 | second,
          })
        })
        .collect(),
    ),
    _ => Err(VistaError::DemMetadataMissing(format!(
      "TIFF tag {tag} has an unsupported floating point type."
    ))),
  }
}

fn entry_ascii(bytes: &[u8], entries: &HashMap<u16, IfdEntry>, tag: u16) -> Option<String> {
  let entry = entries.get(&tag)?;

  if entry.field_type != 2 {
    return None;
  }

  let raw = entry_raw_value(bytes, entry).ok()?;
  Some(String::from_utf8_lossy(raw).to_string())
}

/// An entry's value bytes: inline when they fit in four bytes, otherwise
/// at its offset in the buffer.
fn entry_raw_value<'a>(bytes: &'a [u8], entry: &'a IfdEntry) -> VistaResult<&'a [u8]> {
  let type_size: usize = match entry.field_type {
    1 | 2 => 1,
    3 => 2,
    4 | 11 => 4,
    5 | 12 => 8,
    _ => {
      return Err(VistaError::DemMetadataMissing(format!(
        "unsupported TIFF field type {}.",
        entry.field_type
      )));
    }
  };
  let byte_count = (entry.count as usize)
    .checked_mul(type_size)
    .ok_or_else(|| {
      VistaError::DemMetadataMissing("a TIFF entry's count is too large.".to_string())
    })?;

  if byte_count <= 4 {
    return Ok(&entry.inline_value[..byte_count]);
  }

  slice(bytes, entry.value_offset as usize, byte_count)
}

fn read_u16(bytes: &[u8], offset: usize, endian: Endian) -> VistaResult<u16> {
  Ok(read_chunk_u16(slice(bytes, offset, 2)?, endian))
}

fn read_u32(bytes: &[u8], offset: usize, endian: Endian) -> VistaResult<u32> {
  Ok(read_chunk_u32(slice(bytes, offset, 4)?, endian))
}

fn read_chunk_u16(chunk: &[u8], endian: Endian) -> u16 {
  let array = [chunk[0], chunk[1]];

  match endian {
    Endian::Little => u16::from_le_bytes(array),
    Endian::Big => u16::from_be_bytes(array),
  }
}

fn read_chunk_u32(chunk: &[u8], endian: Endian) -> u32 {
  let array = [chunk[0], chunk[1], chunk[2], chunk[3]];

  match endian {
    Endian::Little => u32::from_le_bytes(array),
    Endian::Big => u32::from_be_bytes(array),
  }
}

fn array3(values: &[f64]) -> Option<[f64; 3]> {
  values.get(..3)?.try_into().ok()
}

fn array6(values: &[f64]) -> Option<[f64; 6]> {
  values.get(..6)?.try_into().ok()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rejects_non_tiff_header() {
    let result = decode_geotiff(&[0, 1, 2], &DemLoadOptions::default());

    assert!(result.is_err());
  }
}
