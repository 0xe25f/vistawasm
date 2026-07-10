use js_sys::{ArrayBuffer, Uint8Array};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use vista_types::{
  AtmosphereOptions, CameraOptions, CloudsOptions, DebugView, DemLoadOptions, FloraOptions,
  FractalTerrainOptions, GrassOptions, MistOptions, RawHeightmapOptions, RenderQualityOptions,
  SunOptions, WaterOptions,
};

use crate::config::VistaEngineConfig;
use crate::engine::EngineCore;
use crate::errors::VistaError;

/// Prepare shared browser error handling for VistaWASM.
#[wasm_bindgen(js_name = initialiseVistaWasm)]
pub fn initialise_vista_wasm() {
  console_error_panic_hook::set_once();
}

/// Browser-facing VistaWASM engine.
#[wasm_bindgen]
pub struct VistaEngine {
  inner: Option<EngineCore>,
}

#[wasm_bindgen]
impl VistaEngine {
  /// Create a new engine for a browser canvas.
  #[wasm_bindgen(js_name = create)]
  pub async fn create(canvas: HtmlCanvasElement, options: JsValue) -> Result<VistaEngine, JsValue> {
    console_error_panic_hook::set_once();
    let config = VistaEngineConfig::from_js(options)?;
    let core = EngineCore::new(canvas, config)
      .await
      .map_err(|error| error.to_js_value())?;

    Ok(Self { inner: Some(core) })
  }

  /// Generate a deterministic fractal terrain from a seed and options.
  #[wasm_bindgen(js_name = generateFractal)]
  pub async fn generate_fractal(&mut self, options: JsValue) -> Result<JsValue, JsValue> {
    let options = from_js::<FractalTerrainOptions>(options)?;
    let handle = self
      .core_mut()?
      .generate_fractal(options)
      .await
      .map_err(|error| error.to_js_value())?;

    to_js(&handle)
  }

  /// Load an uncompressed GeoTIFF DEM from an ArrayBuffer.
  #[wasm_bindgen(js_name = loadDemFromArrayBuffer)]
  pub async fn load_dem_from_array_buffer(
    &mut self,
    buffer: ArrayBuffer,
    options: JsValue,
  ) -> Result<JsValue, JsValue> {
    let bytes = Uint8Array::new(&buffer).to_vec();
    let options = if options.is_undefined() || options.is_null() {
      DemLoadOptions::default()
    } else {
      from_js::<DemLoadOptions>(options)?
    };
    let handle = self
      .core_mut()?
      .load_dem_from_array_buffer(&bytes, options)
      .await
      .map_err(|error| error.to_js_value())?;

    to_js(&handle)
  }

  /// Load a raw heightmap from an ArrayBuffer.
  #[wasm_bindgen(js_name = loadRawHeightmap)]
  pub async fn load_raw_heightmap(
    &mut self,
    buffer: ArrayBuffer,
    options: JsValue,
  ) -> Result<JsValue, JsValue> {
    let bytes = Uint8Array::new(&buffer).to_vec();
    let options = from_js::<RawHeightmapOptions>(options)?;
    let handle = self
      .core_mut()?
      .load_raw_heightmap(&bytes, options)
      .await
      .map_err(|error| error.to_js_value())?;

    to_js(&handle)
  }

  /// Replace the camera controls.
  #[wasm_bindgen(js_name = setCamera)]
  pub fn set_camera(&mut self, camera: JsValue) -> Result<(), JsValue> {
    let camera = from_js::<CameraOptions>(camera)?;
    self
      .core_mut()?
      .set_camera(camera)
      .map_err(|error| error.to_js_value())
  }

  /// Replace sun controls.
  #[wasm_bindgen(js_name = setSun)]
  pub fn set_sun(&mut self, sun: JsValue) -> Result<(), JsValue> {
    let sun = from_js::<SunOptions>(sun)?;
    self
      .core_mut()?
      .set_sun(sun)
      .map_err(|error| error.to_js_value())
  }

  /// Replace atmosphere controls.
  #[wasm_bindgen(js_name = setAtmosphere)]
  pub fn set_atmosphere(&mut self, atmosphere: JsValue) -> Result<(), JsValue> {
    let atmosphere = from_js::<AtmosphereOptions>(atmosphere)?;
    self
      .core_mut()?
      .set_atmosphere(atmosphere)
      .map_err(|error| error.to_js_value())
  }

  /// Replace water controls.
  #[wasm_bindgen(js_name = setWater)]
  pub fn set_water(&mut self, water: JsValue) -> Result<(), JsValue> {
    let water = from_js::<WaterOptions>(water)?;
    self
      .core_mut()?
      .set_water(water)
      .map_err(|error| error.to_js_value())
  }

  /// Replace flora controls.
  #[wasm_bindgen(js_name = setFlora)]
  pub fn set_flora(&mut self, flora: JsValue) -> Result<(), JsValue> {
    let flora = from_js::<FloraOptions>(flora)?;
    self
      .core_mut()?
      .set_flora(flora)
      .map_err(|error| error.to_js_value())
  }

  /// Replace grass controls.
  #[wasm_bindgen(js_name = setGrass)]
  pub fn set_grass(&mut self, grass: JsValue) -> Result<(), JsValue> {
    let grass = from_js::<GrassOptions>(grass)?;
    self
      .core_mut()?
      .set_grass(grass)
      .map_err(|error| error.to_js_value())
  }

  /// Replace cloud controls.
  #[wasm_bindgen(js_name = setClouds)]
  pub fn set_clouds(&mut self, clouds: JsValue) -> Result<(), JsValue> {
    let clouds = from_js::<CloudsOptions>(clouds)?;
    self
      .core_mut()?
      .set_clouds(clouds)
      .map_err(|error| error.to_js_value())
  }

  /// Replace mist/ground-fog controls.
  #[wasm_bindgen(js_name = setMist)]
  pub fn set_mist(&mut self, mist: JsValue) -> Result<(), JsValue> {
    let mist = from_js::<MistOptions>(mist)?;
    self
      .core_mut()?
      .set_mist(mist)
      .map_err(|error| error.to_js_value())
  }

  /// Replace render quality controls.
  #[wasm_bindgen(js_name = setRenderQuality)]
  pub fn set_render_quality(&mut self, quality: JsValue) -> Result<(), JsValue> {
    let quality = from_js::<RenderQualityOptions>(quality)?;
    self
      .core_mut()?
      .set_render_quality(quality)
      .map_err(|error| error.to_js_value())
  }

  /// Set the active debug view.
  #[wasm_bindgen(js_name = setDebugView)]
  pub fn set_debug_view(&mut self, debug_view: JsValue) -> Result<(), JsValue> {
    let debug_view = from_js::<DebugView>(debug_view)?;
    self
      .core_mut()?
      .set_debug_view(debug_view)
      .map_err(|error| error.to_js_value())
  }

  /// Render one frame.
  #[wasm_bindgen(js_name = renderOnce)]
  pub fn render_once(&mut self) -> Result<JsValue, JsValue> {
    let stats = self
      .core_mut()?
      .render_once()
      .map_err(|error| error.to_js_value())?;

    to_js(&stats)
  }

  /// Resize the render surface.
  pub fn resize(
    &mut self,
    width: u32,
    height: u32,
    device_pixel_ratio: Option<f32>,
  ) -> Result<(), JsValue> {
    self
      .core_mut()?
      .resize(width, height, device_pixel_ratio)
      .map_err(|error| error.to_js_value())
  }

  /// Export the active heightmap as little-endian `f32` bytes.
  #[wasm_bindgen(js_name = exportHeightmap)]
  pub fn export_heightmap(&self) -> Result<Uint8Array, JsValue> {
    let bytes = self
      .core_ref()?
      .export_heightmap()
      .map_err(|error| error.to_js_value())?;

    Ok(Uint8Array::from(bytes.as_slice()))
  }

  /// Return current render statistics.
  #[wasm_bindgen(js_name = getStats)]
  pub fn get_stats(&self) -> Result<JsValue, JsValue> {
    to_js(&self.core_ref()?.stats())
  }

  /// Release GPU resources and terrain data.
  pub fn dispose(&mut self) -> Result<(), JsValue> {
    if let Some(mut core) = self.inner.take() {
      core.dispose().map_err(|error| error.to_js_value())?;
    }

    Ok(())
  }
}

impl VistaEngine {
  fn core_mut(&mut self) -> Result<&mut EngineCore, JsValue> {
    self
      .inner
      .as_mut()
      .ok_or_else(|| VistaError::EngineDisposed.to_js_value())
  }

  fn core_ref(&self) -> Result<&EngineCore, JsValue> {
    self
      .inner
      .as_ref()
      .ok_or_else(|| VistaError::EngineDisposed.to_js_value())
  }
}

fn from_js<T>(value: JsValue) -> Result<T, JsValue>
where
  T: serde::de::DeserializeOwned,
{
  serde_wasm_bindgen::from_value(value)
    .map_err(|error| VistaError::options(error.to_string()).to_js_value())
}

fn to_js<T>(value: &T) -> Result<JsValue, JsValue>
where
  T: serde::Serialize,
{
  serde_wasm_bindgen::to_value(value)
    .map_err(|error| VistaError::internal(error.to_string()).to_js_value())
}
