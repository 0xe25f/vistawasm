//! DEM and raw heightmap ingestion.

pub mod geotiff;
pub mod metadata;
pub mod raw_heightmap;
pub mod stream;

pub use geotiff::decode_geotiff;
pub use raw_heightmap::decode_raw_heightmap;
