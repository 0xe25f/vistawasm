use thiserror::Error;
use vista_types::VistaErrorCode;

/// Result type used across VistaWASM.
pub type VistaResult<T> = Result<T, VistaError>;

/// Internal error type with stable public error codes.
#[derive(Debug, Error)]
pub enum VistaError {
  /// WebGPU is unavailable.
  #[error("WebGPU is not available in this browser. VistaWASM requires WebGPU and cannot fall back to WebGL2.")]
  WebGpuUnavailable,
  /// The browser refused to provide a WebGPU device.
  #[error("The browser could not create a WebGPU device for VistaWASM.")]
  WebGpuDeviceRequestFailed,
  /// The active GPU device was lost.
  #[error("The WebGPU device was lost. Stop rendering and create a new VistaWASM engine when the browser allows recovery.")]
  WebGpuDeviceLost,
  /// The host canvas is unusable.
  #[error("The supplied canvas cannot be used for VistaWASM rendering.")]
  CanvasInvalid,
  /// Public options failed validation.
  #[error("Options are not valid: {0}")]
  OptionsInvalid(String),
  /// Terrain generation failed.
  #[error("Terrain generation failed: {0}")]
  TerrainGenerationFailed(String),
  /// DEM fetch failed in the host wrapper.
  #[error("Could not fetch DEM data. {0}")]
  DemFetchFailed(String),
  /// DEM format is unsupported.
  #[error("DEM format is not supported: {0}")]
  DemFormatUnsupported(String),
  /// DEM metadata is missing or contradictory.
  #[error("DEM metadata is missing or contradictory: {0}")]
  DemMetadataMissing(String),
  /// The request exceeds GPU limits.
  #[error("The request exceeds the active GPU limits: {0}")]
  GpuLimitExceeded(String),
  /// The engine was used after disposal.
  #[error("The VistaWASM engine has been disposed.")]
  EngineDisposed,
  /// Unexpected internal fault.
  #[error("VistaWASM encountered an internal fault: {0}")]
  InternalError(String),
}

impl VistaError {
  /// Return the stable public error code.
  pub const fn code(&self) -> VistaErrorCode {
    match self {
      Self::WebGpuUnavailable => VistaErrorCode::WebGpuUnavailable,
      Self::WebGpuDeviceRequestFailed => VistaErrorCode::WebGpuDeviceRequestFailed,
      Self::WebGpuDeviceLost => VistaErrorCode::WebGpuDeviceLost,
      Self::CanvasInvalid => VistaErrorCode::CanvasInvalid,
      Self::OptionsInvalid(_) => VistaErrorCode::OptionsInvalid,
      Self::TerrainGenerationFailed(_) => VistaErrorCode::TerrainGenerationFailed,
      Self::DemFetchFailed(_) => VistaErrorCode::DemFetchFailed,
      Self::DemFormatUnsupported(_) => VistaErrorCode::DemFormatUnsupported,
      Self::DemMetadataMissing(_) => VistaErrorCode::DemMetadataMissing,
      Self::GpuLimitExceeded(_) => VistaErrorCode::GpuLimitExceeded,
      Self::EngineDisposed => VistaErrorCode::EngineDisposed,
      Self::InternalError(_) => VistaErrorCode::InternalError,
    }
  }

  /// Create an options validation error.
  pub fn options(message: impl Into<String>) -> Self {
    Self::OptionsInvalid(message.into())
  }

  /// Create an internal fault.
  pub fn internal(message: impl Into<String>) -> Self {
    Self::InternalError(message.into())
  }

  #[cfg(target_arch = "wasm32")]
  /// Convert the error into a JavaScript error object with stable fields.
  pub fn to_js_value(&self) -> wasm_bindgen::JsValue {
    use js_sys::{Object, Reflect};
    use wasm_bindgen::JsValue;

    let error = js_sys::Error::new(&self.to_string());
    error.set_name("VistaWasmError");
    let object: Object = error.into();
    let _ = Reflect::set(
      &object,
      &JsValue::from_str("code"),
      &JsValue::from_str(self.code().as_str()),
    );
    let _ = Reflect::set(&object, &JsValue::from_str("details"), &JsValue::NULL);
    object.into()
  }
}
