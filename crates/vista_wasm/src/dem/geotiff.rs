use std::collections::HashMap;

use vista_types::{DemLoadOptions, GeospatialMetadata, TerrainMetadata};

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

/// Decode an uncompressed GeoTIFF into a heightmap.
pub fn decode_geotiff(bytes: &[u8], options: &DemLoadOptions) -> VistaResult<HeightMap> {
  let endian = parse_endian(bytes)?;
  let magic = read_u16(bytes, 2, endian)?;

  if magic != 42 {
    return Err(VistaError::DemFormatUnsupported(
      "only classic TIFF files with magic value 42 are supported.".to_string(),
    ));
  }

  let ifd_offset = read_u32(bytes, 4, endian)? as usize;
  let entries = read_ifd(bytes, ifd_offset, endian)?;
  let width = entry_u32(bytes, &entries, TAG_IMAGE_WIDTH, endian)? as u32;
  let height = entry_u32(bytes, &entries, TAG_IMAGE_LENGTH, endian)? as u32;
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

  let bits_per_sample = entry_u32(bytes, &entries, TAG_BITS_PER_SAMPLE, endian)? as u16;
  let sample_format = entry_u32(bytes, &entries, TAG_SAMPLE_FORMAT, endian).unwrap_or(1) as u16;
  let strip_offsets = entry_values_u32(bytes, &entries, TAG_STRIP_OFFSETS, endian)?;
  let strip_byte_counts = entry_values_u32(bytes, &entries, TAG_STRIP_BYTE_COUNTS, endian)?;

  if strip_offsets.len() != strip_byte_counts.len() {
    return Err(VistaError::DemMetadataMissing(
      "strip offset and byte count arrays have different lengths.".to_string(),
    ));
  }

  let no_data_value = entry_ascii(bytes, &entries, TAG_GDAL_NODATA, endian)
    .and_then(|text| text.trim_matches(char::from(0)).trim().parse::<f32>().ok());
  let pixel_scale = entry_values_f64(bytes, &entries, TAG_MODEL_PIXEL_SCALE, endian)
    .ok()
    .and_then(|values| array3(values.as_slice()));
  let tiepoint = entry_values_f64(bytes, &entries, TAG_MODEL_TIEPOINT, endian)
    .ok()
    .and_then(|values| array6(values.as_slice()));
  let warnings = geo_warnings(&entries);
  let mut heights = Vec::with_capacity(width as usize * height as usize);
  let mut no_data = Vec::with_capacity(width as usize * height as usize);

  for (offset, byte_count) in strip_offsets.iter().zip(strip_byte_counts.iter()) {
    let start = *offset as usize;
    let end = start + *byte_count as usize;

    if end > bytes.len() {
      return Err(VistaError::DemMetadataMissing(
        "GeoTIFF strip points outside the supplied buffer.".to_string(),
      ));
    }

    decode_samples(
      &bytes[start..end],
      endian,
      bits_per_sample,
      sample_format,
      no_data_value,
      options.vertical_scale.unwrap_or(1.0),
      &mut heights,
      &mut no_data,
    )?;
  }

  let expected = width as usize * height as usize;

  if heights.len() < expected {
    return Err(VistaError::DemMetadataMissing(
      "GeoTIFF contains fewer samples than width and height require.".to_string(),
    ));
  }

  heights.truncate(expected);
  no_data.truncate(expected);

  let mut metadata = TerrainMetadata {
    width,
    height,
    metres_per_sample: pixel_scale.map(|scale| scale[0] as f32).unwrap_or(1.0),
    vertical_scale: options.vertical_scale.unwrap_or(1.0),
    sea_level_metres: 0.0,
    source: "geotiff".to_string(),
    generator_version: "vistawasm-geotiff-0.1.0".to_string(),
    geospatial: Some(GeospatialMetadata {
      metres_per_sample_x: pixel_scale.map(|scale| scale[0] as f32),
      metres_per_sample_y: pixel_scale.map(|scale| scale[1] as f32),
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

  HeightMap::from_values(width, height, heights, no_data, metadata)
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
  let mut entries = HashMap::new();

  for index in 0..count {
    let entry_offset = offset + 2 + index * 12;
    let tag = read_u16(bytes, entry_offset, endian)?;
    let field_type = read_u16(bytes, entry_offset + 2, endian)?;
    let count = read_u32(bytes, entry_offset + 4, endian)?;
    let value_offset = read_u32(bytes, entry_offset + 8, endian)?;
    let inline_value = [
      bytes[entry_offset + 8],
      bytes[entry_offset + 9],
      bytes[entry_offset + 10],
      bytes[entry_offset + 11],
    ];

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

fn decode_samples(
  bytes: &[u8],
  endian: Endian,
  bits_per_sample: u16,
  sample_format: u16,
  no_data_value: Option<f32>,
  vertical_scale: f32,
  heights: &mut Vec<f32>,
  no_data: &mut Vec<bool>,
) -> VistaResult<()> {
  let sample_size = (bits_per_sample / 8) as usize;

  if sample_size == 0 {
    return Err(VistaError::DemFormatUnsupported(
      "GeoTIFF BitsPerSample must be 16 or 32.".to_string(),
    ));
  }

  for chunk in bytes.chunks_exact(sample_size) {
    let value = match (bits_per_sample, sample_format) {
      (16, 1) => read_chunk_u16(chunk, endian) as f32,
      (16, 2) => read_chunk_i16(chunk, endian) as f32,
      (32, 1) => read_chunk_u32(chunk, endian) as f32,
      (32, 2) => read_chunk_i32(chunk, endian) as f32,
      (32, 3) => read_chunk_f32(chunk, endian),
      _ => {
        return Err(VistaError::DemFormatUnsupported(
          "GeoTIFF samples must be uint16, int16, uint32, int32, or float32.".to_string(),
        ));
      }
    };
    let is_no_data = no_data_value.is_some_and(|marker| (value - marker).abs() <= f32::EPSILON);

    heights.push(if is_no_data {
      0.0
    } else {
      value * vertical_scale
    });
    no_data.push(is_no_data);
  }

  Ok(())
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
  entries.get(&tag).ok_or_else(|| {
    VistaError::DemMetadataMissing(format!("required TIFF tag {tag} is missing."))
  })?;
  let values = entry_values_u32(bytes, entries, tag, endian)?;

  values
    .first()
    .copied()
    .ok_or_else(|| VistaError::DemMetadataMissing(format!("TIFF tag {tag} has no value.")))
}

fn entry_values_u32(
  bytes: &[u8],
  entries: &HashMap<u16, IfdEntry>,
  tag: u16,
  endian: Endian,
) -> VistaResult<Vec<u32>> {
  let entry = entries.get(&tag).ok_or_else(|| {
    VistaError::DemMetadataMissing(format!("required TIFF tag {tag} is missing."))
  })?;
  let raw = entry_raw_value(bytes, entry)?;
  let mut result = Vec::with_capacity(entry.count as usize);

  match entry.field_type {
    3 => {
      for chunk in raw.chunks_exact(2).take(entry.count as usize) {
        result.push(read_chunk_u16(chunk, endian) as u32);
      }
    }
    4 => {
      for chunk in raw.chunks_exact(4).take(entry.count as usize) {
        result.push(read_chunk_u32(chunk, endian));
      }
    }
    _ => {
      return Err(VistaError::DemMetadataMissing(format!(
        "TIFF tag {tag} has an unsupported numeric type."
      )));
    }
  }

  Ok(result)
}

fn entry_values_f64(
  bytes: &[u8],
  entries: &HashMap<u16, IfdEntry>,
  tag: u16,
  endian: Endian,
) -> VistaResult<Vec<f64>> {
  let entry = entries.get(&tag).ok_or_else(|| {
    VistaError::DemMetadataMissing(format!("required TIFF tag {tag} is missing."))
  })?;
  let raw = entry_raw_value(bytes, entry)?;
  let mut result = Vec::with_capacity(entry.count as usize);

  match entry.field_type {
    5 => {
      for chunk in raw.chunks_exact(8).take(entry.count as usize) {
        let numerator = read_chunk_u32(&chunk[0..4], endian);
        let denominator = read_chunk_u32(&chunk[4..8], endian).max(1);
        result.push(numerator as f64 / denominator as f64);
      }
    }
    12 => {
      for chunk in raw.chunks_exact(8).take(entry.count as usize) {
        let array = [
          chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
        ];
        result.push(match endian {
          Endian::Little => f64::from_le_bytes(array),
          Endian::Big => f64::from_be_bytes(array),
        });
      }
    }
    _ => {
      return Err(VistaError::DemMetadataMissing(format!(
        "TIFF tag {tag} has an unsupported floating point type."
      )));
    }
  }

  Ok(result)
}

fn entry_ascii(
  bytes: &[u8],
  entries: &HashMap<u16, IfdEntry>,
  tag: u16,
  _endian: Endian,
) -> Option<String> {
  let entry = entries.get(&tag)?;

  if entry.field_type != 2 {
    return None;
  }

  let raw = entry_raw_value(bytes, entry).ok()?;
  Some(String::from_utf8_lossy(&raw).to_string())
}

fn entry_raw_value(bytes: &[u8], entry: &IfdEntry) -> VistaResult<Vec<u8>> {
  let type_size = match entry.field_type {
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
  let byte_count = entry.count as usize * type_size;

  if byte_count <= 4 {
    return Ok(entry.inline_value[..byte_count].to_vec());
  }

  let start = entry.value_offset as usize;
  let end = start + byte_count;

  if end > bytes.len() {
    return Err(VistaError::DemMetadataMissing(
      "TIFF entry points outside the supplied buffer.".to_string(),
    ));
  }

  Ok(bytes[start..end].to_vec())
}

fn read_u16(bytes: &[u8], offset: usize, endian: Endian) -> VistaResult<u16> {
  if offset + 2 > bytes.len() {
    return Err(VistaError::DemMetadataMissing(
      "TIFF u16 read is outside the supplied buffer.".to_string(),
    ));
  }

  Ok(read_chunk_u16(&bytes[offset..offset + 2], endian))
}

fn read_u32(bytes: &[u8], offset: usize, endian: Endian) -> VistaResult<u32> {
  if offset + 4 > bytes.len() {
    return Err(VistaError::DemMetadataMissing(
      "TIFF u32 read is outside the supplied buffer.".to_string(),
    ));
  }

  Ok(read_chunk_u32(&bytes[offset..offset + 4], endian))
}

fn read_chunk_u16(chunk: &[u8], endian: Endian) -> u16 {
  let array = [chunk[0], chunk[1]];

  match endian {
    Endian::Little => u16::from_le_bytes(array),
    Endian::Big => u16::from_be_bytes(array),
  }
}

fn read_chunk_i16(chunk: &[u8], endian: Endian) -> i16 {
  let array = [chunk[0], chunk[1]];

  match endian {
    Endian::Little => i16::from_le_bytes(array),
    Endian::Big => i16::from_be_bytes(array),
  }
}

fn read_chunk_u32(chunk: &[u8], endian: Endian) -> u32 {
  let array = [chunk[0], chunk[1], chunk[2], chunk[3]];

  match endian {
    Endian::Little => u32::from_le_bytes(array),
    Endian::Big => u32::from_be_bytes(array),
  }
}

fn read_chunk_i32(chunk: &[u8], endian: Endian) -> i32 {
  let array = [chunk[0], chunk[1], chunk[2], chunk[3]];

  match endian {
    Endian::Little => i32::from_le_bytes(array),
    Endian::Big => i32::from_be_bytes(array),
  }
}

fn read_chunk_f32(chunk: &[u8], endian: Endian) -> f32 {
  let array = [chunk[0], chunk[1], chunk[2], chunk[3]];

  match endian {
    Endian::Little => f32::from_le_bytes(array),
    Endian::Big => f32::from_be_bytes(array),
  }
}

fn array3(values: &[f64]) -> Option<[f64; 3]> {
  if values.len() < 3 {
    return None;
  }

  Some([values[0], values[1], values[2]])
}

fn array6(values: &[f64]) -> Option<[f64; 6]> {
  if values.len() < 6 {
    return None;
  }

  Some([
    values[0], values[1], values[2], values[3], values[4], values[5],
  ])
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
