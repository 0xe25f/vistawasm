//! Terrain storage, generation, erosion, normals, materials, and clipmap data.

pub mod biomes;
pub mod clipmap;
pub mod drainage;
pub mod erosion;
pub mod fractal;
pub mod heightmap;
pub mod materials;
pub mod noise;
pub mod normals;

pub use fractal::{generate_fractal_heightmap, generate_fractal_heightmap_base};
pub use heightmap::HeightMap;
