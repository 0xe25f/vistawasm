//! VistaWASM engine crate.
//!
//! The crate keeps terrain generation, DEM parsing, camera maths, and renderer
//! ownership in Rust. JavaScript receives a small `wasm-bindgen` surface on
//! browser builds and plain Rust APIs for tests.

pub mod camera;
pub mod config;
pub mod dem;
pub mod engine;
pub mod errors;
pub mod maths;
pub mod render;
pub mod terrain;

#[cfg(target_arch = "wasm32")]
pub mod api;

pub use errors::{VistaError, VistaResult};
