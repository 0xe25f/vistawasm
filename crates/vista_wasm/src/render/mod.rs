//! WebGPU renderer modules.

pub mod atmosphere;
pub mod debug;
#[cfg(target_arch = "wasm32")]
pub mod erosion_compute;
pub mod flora;
pub mod grass;
#[cfg(target_arch = "wasm32")]
pub mod gpu;
pub mod pipelines;
pub mod terrain_mesh;
pub mod water;
