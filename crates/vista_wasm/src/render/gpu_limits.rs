//! The buffer limits the engine asks the GPU for, and checks against.
//!
//! WebGPU devices start with small default limits (256 MiB buffers and
//! 128 MiB storage bindings) whatever the adapter can do. The engine asks
//! for what the adapter offers, never less than the defaults, and checks
//! its largest buffers against the device's limits before creating them,
//! so a terrain too large for the GPU falls back or fails with a clear
//! error instead of a GPU validation error.

use crate::errors::{VistaError, VistaResult};

/// WebGPU's default `maxBufferSize`.
pub const DEFAULT_MAX_BUFFER_SIZE: u64 = 256 << 20;
/// WebGPU's default `maxStorageBufferBindingSize`.
pub const DEFAULT_MAX_STORAGE_BINDING: u64 = 128 << 20;

/// The buffer limits of a device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferLimits {
  /// Largest buffer, in bytes.
  pub max_buffer_size: u64,
  /// Largest storage buffer binding, in bytes.
  pub max_storage_binding: u64,
}

impl Default for BufferLimits {
  fn default() -> Self {
    Self {
      max_buffer_size: DEFAULT_MAX_BUFFER_SIZE,
      max_storage_binding: DEFAULT_MAX_STORAGE_BINDING,
    }
  }
}

impl BufferLimits {
  /// The limits to request from an adapter offering `adapter`: all of
  /// it, with the defaults as the floor.
  pub fn requested(adapter: BufferLimits) -> Self {
    Self {
      max_buffer_size: adapter.max_buffer_size.max(DEFAULT_MAX_BUFFER_SIZE),
      max_storage_binding: adapter.max_storage_binding.max(DEFAULT_MAX_STORAGE_BINDING),
    }
  }

  /// Whether a storage buffer of `bytes` bytes, bound whole, fits.
  pub fn storage_fits(&self, bytes: u64) -> bool {
    bytes <= self.max_buffer_size && bytes <= self.max_storage_binding
  }
}

/// Bytes per cell of GPU erosion's largest buffers: the outflow flux and
/// the velocity, four `f32`s each.
pub const EROSION_BYTES_PER_CELL: u64 = 16;

/// Check that GPU erosion of a `size` x `size` grid fits `limits`. Its
/// flux and velocity buffers take 16 bytes a cell: 64 MiB at 2048, the
/// largest terrain, within WebGPU's 128 MiB default binding limit. When
/// they do not fit, erosion runs on the CPU instead (see
/// `EngineCore::generate_fractal_map`).
pub fn check_erosion(size: u32, limits: &BufferLimits) -> VistaResult<()> {
  let bytes = u64::from(size) * u64::from(size) * EROSION_BYTES_PER_CELL;

  if limits.storage_fits(bytes) {
    return Ok(());
  }

  Err(VistaError::GpuLimitExceeded(format!(
    "GPU erosion at {size} x {size} needs {bytes}-byte storage buffers, but this GPU allows {} bytes a buffer and {} a binding.",
    limits.max_buffer_size, limits.max_storage_binding
  )))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn asks_for_what_the_adapter_offers_but_never_less_than_the_defaults() {
    let large = BufferLimits {
      max_buffer_size: 4 << 30,
      max_storage_binding: 2 << 30,
    };
    assert_eq!(BufferLimits::requested(large), large);
    let small = BufferLimits {
      max_buffer_size: 64 << 20,
      max_storage_binding: 64 << 20,
    };
    assert_eq!(BufferLimits::requested(small), BufferLimits::default());
  }

  #[test]
  fn erosion_falls_back_when_its_buffers_pass_the_limits() {
    let defaults = BufferLimits::default();
    // 2048: 64 MiB buffers, within the defaults.
    assert!(check_erosion(2048, &defaults).is_ok());
    // 4096: 256 MiB, past the 128 MiB binding default.
    let error = check_erosion(4096, &defaults).unwrap_err();
    assert_eq!(error.code(), vista_types::VistaErrorCode::GpuLimitExceeded);
    assert!(error.to_string().contains("268435456-byte"), "{error}");
    // A large adapter takes 8192 (1 GiB) whole.
    let large = BufferLimits {
      max_buffer_size: 2 << 30,
      max_storage_binding: 2 << 30,
    };
    assert!(check_erosion(8192, &large).is_ok());
    // Its buffer limit alone is not enough.
    let narrow = BufferLimits {
      max_buffer_size: 2 << 30,
      max_storage_binding: 512 << 20,
    };
    assert!(check_erosion(8192, &narrow).is_err());
  }
}
