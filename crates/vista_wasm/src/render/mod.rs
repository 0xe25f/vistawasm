//! WebGPU renderer modules.

pub mod atmosphere;
pub mod debug;
#[cfg(target_arch = "wasm32")]
pub mod erosion_compute;
pub mod flora;
#[cfg(target_arch = "wasm32")]
pub mod gpu;
pub mod grass;
pub mod pipelines;
pub mod shaders;
pub mod shadow_math;
pub mod terrain_mesh;
#[cfg(target_arch = "wasm32")]
pub mod textures;
pub mod tree_models;
pub mod water;
