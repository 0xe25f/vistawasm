//! Terrain storage, generation, erosion, normals, materials, and clipmap data.

pub mod biomes;
pub mod channels;
pub mod clipmap;
pub mod drainage;
pub mod erosion;
pub mod fractal;
pub mod glaciers;
pub mod heightmap;
pub mod hydrology;
pub mod landforms;
pub mod materials;
pub mod noise;
pub mod normals;
#[cfg(test)]
mod realism_tests;
pub mod stream_power;
pub mod tectonics;

pub use fractal::{
  finish_fractal_heightmap, generate_fractal_heightmap, generate_fractal_heightmap_base,
  generate_fractal_heightmap_base_with_progress, generate_fractal_heightmap_with_progress,
};
pub use heightmap::HeightMap;
