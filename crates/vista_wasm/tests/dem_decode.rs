use vista_types::{ByteOrder, RawHeightmapOptions, RawSampleFormat};
use vista_wasm::dem::{decode_geotiff, decode_raw_heightmap};

#[test]
fn raw_heightmap_decodes_big_endian_uint16() {
  let bytes = [0_u8, 10, 0, 20];
  let map = decode_raw_heightmap(
    &bytes,
    &RawHeightmapOptions {
      width: 2,
      height: 1,
      sample_format: RawSampleFormat::Uint16,
      byte_order: Some(ByteOrder::BigEndian),
      metres_per_sample: 30.0,
      height_scale_metres: 1.0,
      no_data_value: None,
      sea_level_metres: None,
    },
  )
  .unwrap();

  assert_eq!(map.heights, vec![10.0, 20.0]);
}

#[test]
fn geotiff_rejects_short_header() {
  let result = decode_geotiff(&[1, 2, 3], &Default::default());

  assert!(result.is_err());
}
