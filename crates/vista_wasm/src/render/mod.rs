//! WebGPU renderer modules.

pub mod atmosphere;
pub mod boulders;
pub mod channel_field;
pub mod debug;
#[cfg(target_arch = "wasm32")]
pub mod erosion_compute;
pub mod flora;
#[cfg(target_arch = "wasm32")]
pub mod gpu;
pub mod gpu_limits;
pub mod grass;
pub mod lattice;
pub mod pipelines;
pub mod shaders;
pub mod shadow_math;
pub mod terrain_mesh;
#[cfg(target_arch = "wasm32")]
pub mod textures;
pub mod tree_growth;
pub mod tree_models;
pub mod vegetation;
pub mod water;

#[cfg(test)]
mod cloud_tests;
#[cfg(test)]
mod pack_ice_tests;
