use vista_types::{DemMetadata, GeospatialMetadata, TerrainMetadata};

/// Build terrain metadata from decoded DEM metadata.
pub fn terrain_metadata_from_dem(dem: &DemMetadata, vertical_scale: f32) -> TerrainMetadata {
  let metres_per_sample = dem
    .metres_per_sample_x
    .or(dem.metres_per_sample_y)
    .unwrap_or(1.0);

  TerrainMetadata {
    width: dem.width,
    height: dem.height,
    metres_per_sample,
    vertical_scale,
    sea_level_metres: 0.0,
    min_height_metres: dem.min_height_metres,
    max_height_metres: dem.max_height_metres,
    mean_height_metres: 0.0,
    source: "geotiff".to_string(),
    generator_version: "vistawasm-geotiff-0.1.0".to_string(),
    geospatial: Some(GeospatialMetadata {
      metres_per_sample_x: dem.metres_per_sample_x,
      metres_per_sample_y: dem.metres_per_sample_y,
      projection_name: dem.projection_name.clone(),
      model_tiepoint: None,
      model_pixel_scale: None,
    }),
    warnings: dem.warnings.clone(),
  }
}
