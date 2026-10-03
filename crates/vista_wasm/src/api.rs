use js_sys::{ArrayBuffer, Float32Array, Uint8Array};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use vista_types::{
  AtmosphereOptions, BiomeOptions, CameraOptions, CloudsOptions, DebugView, DemLoadOptions,
  FloraOptions, FractalTerrainOptions, GrassOptions, MistOptions, RawHeightmapOptions,
  RenderQualityOptions, ShadowOptions, SunOptions, SurfaceOptions, TextureTarget, TreeSpeciesKind,
  WaterOptions, WeatherOptions,
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
  /// The dominant weather at the last frame, for `renderFrame`.
  weather: Option<vista_types::WeatherKind>,
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

    Ok(Self {
      inner: Some(core),
      weather: None,
    })
  }

  /// Generate a deterministic fractal terrain from a seed and options.
  /// `on_progress`, when given, is called as `(phase, progress)` while
  /// generation runs.
  #[wasm_bindgen(js_name = generateFractal)]
  pub async fn generate_fractal(
    &mut self,
    options: JsValue,
    on_progress: Option<js_sys::Function>,
  ) -> Result<JsValue, JsValue> {
    let options = from_js::<FractalTerrainOptions>(options)?;
    let mut progress = |phase: &str, value: f32| {
      if let Some(callback) = &on_progress {
        // A throwing listener must not abort generation.
        let _ = callback.call2(
          &JsValue::NULL,
          &JsValue::from_str(phase),
          &JsValue::from_f64(value as f64),
        );
      }
    };
    let handle = self
      .core_mut()?
      .generate_fractal_with_progress(options, &mut progress)
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
    let options = if options.is_undefined() || options.is_null() {
      DemLoadOptions::default()
    } else {
      from_js::<DemLoadOptions>(options)?
    };
    // The file is parsed in place, so it is copied in whole; its length is
    // checked first.
    crate::dem::geotiff::check_geotiff_length(byte_length(&buffer))
      .map_err(|error| error.to_js_value())?;
    let bytes = Uint8Array::new(&buffer).to_vec();
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
    let options = from_js::<RawHeightmapOptions>(options)?;
    let handle = self
      .core_mut()?
      .load_raw_heightmap(&HostBytes(buffer), options)
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

  /// Replace biome controls. Re-bakes terrain materials, trees, and grass.
  #[wasm_bindgen(js_name = setBiomes)]
  pub fn set_biomes(&mut self, biomes: JsValue) -> Result<(), JsValue> {
    let biomes = from_js::<BiomeOptions>(biomes)?;
    self
      .core_mut()?
      .set_biomes(biomes)
      .map_err(|error| error.to_js_value())
  }

  /// Return the biome name at a world position, or `undefined` outside the
  /// terrain.
  #[wasm_bindgen(js_name = biomeAt)]
  pub fn biome_at(&self, x: f32, z: f32) -> Result<JsValue, JsValue> {
    match self.core_ref()?.biome_at(x, z) {
      Some(biome) => to_js(&biome),
      None => Ok(JsValue::UNDEFINED),
    }
  }

  /// Return the mean annual temperature in °C at a world position, or
  /// `undefined` outside the terrain.
  #[wasm_bindgen(js_name = temperatureAt)]
  pub fn temperature_at(&self, x: f32, z: f32) -> Result<Option<f32>, JsValue> {
    Ok(self.core_ref()?.celsius_at(x, z))
  }

  /// Paint rivers and lakes (`data` is `width x height` bytes), or clear
  /// the painted water with `undefined`. Returns a warning when the mask
  /// had to be resampled to the terrain's size.
  #[wasm_bindgen(js_name = setWaterMask)]
  pub fn set_water_mask(
    &mut self,
    width: u32,
    height: u32,
    data: Option<Vec<u8>>,
  ) -> Result<Option<String>, JsValue> {
    let mask = data.map(|data| vista_types::WaterMask {
      width,
      height,
      data,
    });
    self
      .core_mut()?
      .set_water_mask(mask)
      .map_err(|error| error.to_js_value())
  }

  /// Paint biomes (`data` is `width x height` biome indices, 255 where
  /// not painted), or clear them with `undefined`. Returns warnings, one
  /// a line, or an empty string.
  #[wasm_bindgen(js_name = setBiomeMap)]
  pub fn set_biome_map(
    &mut self,
    width: u32,
    height: u32,
    data: Option<Vec<u8>>,
    border: u32,
  ) -> Result<String, JsValue> {
    let map = data.as_deref().map(|data| (width, height, data));
    self
      .core_mut()?
      .set_biome_map(map, border)
      .map(|warnings| warnings.join("\n"))
      .map_err(|error| error.to_js_value())
  }

  /// Scale the trees (`grass` false) or the grass with a density mask
  /// (`data` is `width x height` bytes), or clear it with `undefined`.
  /// Returns a warning when the mask had to be resampled.
  #[wasm_bindgen(js_name = setVegetationMask)]
  pub fn set_vegetation_mask(
    &mut self,
    grass: bool,
    width: u32,
    height: u32,
    data: Option<Vec<u8>>,
  ) -> Result<Option<String>, JsValue> {
    let mask = data.as_deref().map(|data| (width, height, data));
    self
      .core_mut()?
      .set_vegetation_mask(grass, mask)
      .map_err(|error| error.to_js_value())
  }

  /// The loudest river, waterfall, lake shore and surf near a position,
  /// as five numbers each (distance, loudness, x, y, z), with a NaN
  /// distance where there is none.
  #[wasm_bindgen(js_name = getWaterSounds)]
  pub fn get_water_sounds(&self, x: f32, y: f32, z: f32) -> Result<Float32Array, JsValue> {
    let sounds = self.core_ref()?.water_sounds(x, y, z);
    let mut packed = Vec::with_capacity(20);

    for sound in [
      sounds.river,
      sounds.waterfall,
      sounds.lake_shore,
      sounds.surf,
    ] {
      match sound {
        Some(sound) => {
          packed.extend([sound.distance_metres, sound.loudness]);
          packed.extend(sound.position);
        }
        None => packed.extend([f32::NAN; 5]),
      }
    }

    Ok(floats(&packed))
  }

  /// Every waterfall as six numbers (x, y, z where the water lands,
  /// height, width, discharge).
  #[wasm_bindgen(js_name = getWaterfalls)]
  pub fn get_waterfalls(&self) -> Result<Float32Array, JsValue> {
    let packed: Vec<f32> = self
      .core_ref()?
      .waterfalls()
      .iter()
      .flat_map(|fall| {
        let [x, y, z] = fall.position;
        [
          x,
          y,
          z,
          fall.height_metres,
          fall.width_metres,
          fall.discharge_cubic_metres_per_second,
        ]
      })
      .collect();
    Ok(floats(&packed))
  }

  /// Every inflow in use as four numbers (x, y, z where the water enters,
  /// discharge).
  #[wasm_bindgen(js_name = getInflows)]
  pub fn get_inflows(&self) -> Result<Float32Array, JsValue> {
    let packed: Vec<f32> = self
      .core_ref()?
      .inflows()
      .iter()
      .flat_map(|inflow| {
        let [x, y, z] = inflow.position;
        [x, y, z, inflow.discharge_cubic_metres_per_second]
      })
      .collect();
    Ok(floats(&packed))
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
    let core = self
      .inner
      .as_mut()
      .ok_or_else(|| VistaError::EngineDisposed.to_js_value())?;
    let stats = core.render_once().map_err(|error| error.to_js_value())?;

    if stats.weather != self.weather {
      self.weather.clone_from(&stats.weather);
    }

    to_js(stats)
  }

  /// Render one frame without building its stats object, which would be
  /// garbage every frame when nothing reads it. Returns the frame index,
  /// negated when the dominant weather changed since the last frame;
  /// `getStats()` reads the rest.
  #[wasm_bindgen(js_name = renderFrame)]
  pub fn render_frame(&mut self) -> Result<f64, JsValue> {
    let core = self
      .inner
      .as_mut()
      .ok_or_else(|| VistaError::EngineDisposed.to_js_value())?;
    let stats = core.render_once().map_err(|error| error.to_js_value())?;
    let changed = stats.weather != self.weather;

    // Kept only on a change: a custom preset's name would be copied.
    if changed {
      self.weather.clone_from(&stats.weather);
    }

    let index = stats.frame_index as f64;
    Ok(if changed { -index } else { index })
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

  /// Export one map (see `export.rs`), `kind` being its index in
  /// `MapKind::ALL`, at the terrain's own size or, with both given,
  /// `width x height`. Returns `{ width, height, channels, data, units,
  /// scale, range, legendNames, legendColours, metresPerPixel,
  /// seaLevelMetres, generator }`: `data` is a `Float32Array` or a
  /// `Uint8Array`, and the legend's names come one a line with three
  /// colour components each.
  #[wasm_bindgen(js_name = exportMap)]
  pub fn export_map(
    &self,
    kind: u32,
    width: Option<u32>,
    height: Option<u32>,
  ) -> Result<JsValue, JsValue> {
    let kind = *crate::export::MapKind::ALL
      .get(kind as usize)
      .ok_or_else(|| VistaError::options("exportMap kind is not a map kind.").to_js_value())?;

    if width.is_some() != height.is_some() {
      return Err(
        VistaError::options("exportMap size must give both width and height, or neither.")
          .to_js_value(),
      );
    }

    let map = self
      .core_ref()?
      .export_map(kind, width.zip(height).map(|(w, h)| [w, h]))
      .map_err(|error| error.to_js_value())?;
    let encoding = &map.encoding;
    let legend = encoding.legend.unwrap_or_default();
    let names: Vec<&str> = legend.iter().map(|(name, _)| *name).collect();
    let colours: Vec<f32> = legend.iter().flat_map(|(_, colour)| *colour).collect();
    let object = js_sys::Object::new();

    for (name, value) in [
      ("width", JsValue::from(map.width)),
      ("height", JsValue::from(map.height)),
      ("channels", JsValue::from(map.channels)),
      (
        "data",
        match &map.data {
          crate::export::MapData::F32(values) => floats(values).into(),
          crate::export::MapData::U8(values) => Uint8Array::from(values.as_slice()).into(),
        },
      ),
      (
        "units",
        encoding.units.map_or(JsValue::UNDEFINED, JsValue::from_str),
      ),
      (
        "scale",
        encoding.scale.map_or(JsValue::UNDEFINED, JsValue::from),
      ),
      (
        "range",
        encoding
          .range
          .map_or(JsValue::UNDEFINED, |range| floats(&range).into()),
      ),
      ("legendNames", JsValue::from_str(&names.join("\n"))),
      ("legendColours", floats(&colours).into()),
      ("metresPerPixel", floats(&encoding.metres_per_pixel).into()),
      ("seaLevelMetres", JsValue::from(encoding.sea_level_metres)),
      ("generator", JsValue::from_str(&encoding.generator)),
    ] {
      js_sys::Reflect::set(&object, &JsValue::from_str(name), &value)?;
    }

    Ok(object.into())
  }

  /// Export the trees over the map, or in `region` (min x, min z, max x,
  /// max z in metres), packed ten floats a tree (see
  /// `export::TREE_RECORD_FLOATS`).
  #[wasm_bindgen(js_name = exportTrees)]
  pub fn export_trees(
    &self,
    region: Option<Vec<f32>>,
    max_count: Option<u32>,
  ) -> Result<Float32Array, JsValue> {
    let region = match region.as_deref() {
      None => None,
      Some([min_x, min_z, max_x, max_z]) => Some([*min_x, *min_z, *max_x, *max_z]),
      Some(_) => {
        return Err(
          VistaError::options("exportTrees region must hold minX, minZ, maxX and maxZ.")
            .to_js_value(),
        )
      }
    };
    let packed = self
      .core_ref()?
      .export_trees(region, max_count)
      .map_err(|error| error.to_js_value())?;

    Ok(floats(&packed))
  }

  /// Replace weather controls.
  #[wasm_bindgen(js_name = setWeather)]
  pub fn set_weather(&mut self, weather: JsValue) -> Result<(), JsValue> {
    let weather = from_js::<WeatherOptions>(weather)?;
    self
      .core_mut()?
      .set_weather(weather)
      .map_err(|error| error.to_js_value())
  }

  /// Return the current blended weather, or `undefined` when the weather
  /// system is off.
  #[wasm_bindgen(js_name = getWeather)]
  pub fn get_weather(&self) -> Result<JsValue, JsValue> {
    match self.core_ref()?.weather() {
      Some(state) => to_js(&state),
      None => Ok(JsValue::UNDEFINED),
    }
  }

  /// Return every resolved weather preset, built in and custom, as an
  /// object keyed by name.
  #[wasm_bindgen(js_name = getWeatherPresets)]
  pub fn get_weather_presets(&self) -> Result<JsValue, JsValue> {
    let presets = self.core_ref()?.weather_presets();
    let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
    serde::Serialize::serialize(&presets, &serializer)
      .map_err(|error| VistaError::internal(error.to_string()).to_js_value())
  }

  /// Return the weather at a world position, packed as coverage,
  /// precipitation, storminess, humidity, wetness, puddles and snow depth,
  /// or empty with no terrain.
  #[wasm_bindgen(js_name = weatherAt)]
  pub fn weather_at(&self, x: f32, z: f32) -> Result<Float32Array, JsValue> {
    Ok(match self.core_ref()?.weather_at(x, z) {
      Some(local) => floats(&[
        local.coverage,
        local.precipitation,
        local.storminess,
        local.humidity,
        local.wetness,
        local.puddles,
        local.snow_depth,
      ]),
      None => floats(&[]),
    })
  }

  /// Run the weather forward by `seconds` at once.
  #[wasm_bindgen(js_name = advanceWeather)]
  pub fn advance_weather(&mut self, seconds: f32) -> Result<(), JsValue> {
    self
      .core_mut()?
      .advance_weather(seconds)
      .map_err(|error| error.to_js_value())
  }

  /// Replace the time of day controls. The wrapper fills in defaults.
  #[wasm_bindgen(js_name = setTimeOfDay)]
  pub fn set_time_of_day(
    &mut self,
    enabled: bool,
    hours: f32,
    day_length_minutes: f32,
    latitude_degrees: f32,
    day_of_year: u32,
  ) -> Result<(), JsValue> {
    self
      .core_mut()?
      .set_time_of_day(vista_types::TimeOfDayOptions {
        enabled,
        hours,
        day_length_minutes,
        latitude_degrees,
        day_of_year,
      })
      .map_err(|error| error.to_js_value())
  }

  /// Return the time of day, packed as hours, sun azimuth and elevation,
  /// sunrise and sunset (NaN during polar day or night).
  #[wasm_bindgen(js_name = getTimeOfDay)]
  pub fn get_time_of_day(&self) -> Result<Float32Array, JsValue> {
    let time = self.core_ref()?.time_of_day();
    Ok(floats(&[
      time.hours,
      time.sun_azimuth_degrees,
      time.sun_elevation_degrees,
      time.sunrise_hours.unwrap_or(f32::NAN),
      time.sunset_hours.unwrap_or(f32::NAN),
    ]))
  }

  /// Replace shadow controls.
  #[wasm_bindgen(js_name = setShadows)]
  pub fn set_shadows(&mut self, shadows: JsValue) -> Result<(), JsValue> {
    let shadows = from_js::<ShadowOptions>(shadows)?;
    self
      .core_mut()?
      .set_shadows(shadows)
      .map_err(|error| error.to_js_value())
  }

  /// Replace terrain surface controls.
  #[wasm_bindgen(js_name = setSurface)]
  pub fn set_surface(&mut self, surface: JsValue) -> Result<(), JsValue> {
    let surface = from_js::<SurfaceOptions>(surface)?;
    self
      .core_mut()?
      .set_surface(surface)
      .map_err(|error| error.to_js_value())
  }

  /// Replace one species' model. Arrays are copied; see the TypeScript
  /// `TreeModel` type for the layout.
  #[wasm_bindgen(js_name = setTreeModel)]
  #[allow(clippy::too_many_arguments)]
  pub fn set_tree_model(
    &mut self,
    species: JsValue,
    positions: Vec<f32>,
    normals: Vec<f32>,
    uvs: Vec<f32>,
    indices: Vec<u32>,
    texture_layers: Option<Vec<f32>>,
    wind: Option<Vec<f32>>,
  ) -> Result<(), JsValue> {
    let species = from_js::<TreeSpeciesKind>(species)?;
    self
      .core_mut()?
      .set_tree_model(
        species,
        &positions,
        &normals,
        &uvs,
        &indices,
        texture_layers.as_deref(),
        wind.as_deref(),
      )
      .map_err(|error| error.to_js_value())
  }

  /// Restore the procedural model for one species.
  #[wasm_bindgen(js_name = resetTreeModel)]
  pub fn reset_tree_model(&mut self, species: JsValue) -> Result<(), JsValue> {
    let species = from_js::<TreeSpeciesKind>(species)?;
    self
      .core_mut()?
      .reset_tree_model(species)
      .map_err(|error| error.to_js_value())
  }

  /// Replace procedural tree placement with packed instances (nine floats
  /// per tree: x, y, z, scale, rotation, tint, species, dryness and a
  /// ground flag of 0 or 1), or restore procedural placement with
  /// `undefined`.
  #[wasm_bindgen(js_name = setTreeInstances)]
  pub fn set_tree_instances(&mut self, packed: Option<Vec<f32>>) -> Result<(), JsValue> {
    let trees = match packed {
      None => None,
      Some(packed) => Some(
        crate::render::flora::unpack_tree_placements(&packed)
          .map_err(|message| VistaError::options(message).to_js_value())?,
      ),
    };

    self
      .core_mut()?
      .set_tree_instances(trees)
      .map_err(|error| error.to_js_value())
  }

  /// Replace one layer of a texture array with 512 x 512 RGBA8 texels.
  #[wasm_bindgen(js_name = replaceTexture)]
  pub fn replace_texture(
    &mut self,
    target: JsValue,
    layer: u32,
    rgba: Vec<u8>,
  ) -> Result<(), JsValue> {
    let target = from_js::<TextureTarget>(target)?;
    self
      .core_mut()?
      .replace_texture(target, layer, &rgba)
      .map_err(|error| error.to_js_value())
  }

  /// Restore every procedural texture.
  #[wasm_bindgen(js_name = resetTextures)]
  pub fn reset_textures(&mut self) -> Result<(), JsValue> {
    self
      .core_mut()?
      .reset_textures()
      .map_err(|error| error.to_js_value())
  }

  /// GPU errors no error scope caught since the last call, and why the
  /// device was lost (reported once), one a line after the reason's line
  /// (empty while the device is fine), or `undefined` when there is
  /// nothing to report.
  #[wasm_bindgen(js_name = takeGpuEvents)]
  pub fn take_gpu_events(&self) -> Result<Option<String>, JsValue> {
    let (errors, lost) = self.core_ref()?.take_gpu_events();

    if errors.is_empty() && lost.is_none() {
      return Ok(None);
    }

    Ok(Some(
      std::iter::once(lost.unwrap_or_default())
        .chain(errors)
        .collect::<Vec<_>>()
        .join("\n"),
    ))
  }

  /// Check engine options as `create` and the setters would, without
  /// applying them; the wrapper checks a bundle's settings this way
  /// before the scene changes.
  #[wasm_bindgen(js_name = checkOptions)]
  pub fn check_options(&self, options: JsValue) -> Result<(), JsValue> {
    self.core_ref()?;
    VistaEngineConfig::from_js(options).map(|_| ())
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

/// Copy numbers out to JavaScript.
fn floats(values: &[f32]) -> Float32Array {
  Float32Array::from(values)
}

fn from_js<T>(value: JsValue) -> Result<T, JsValue>
where
  T: serde::de::DeserializeOwned,
{
  serde_wasm_bindgen::from_value(value)
    .map_err(|error| VistaError::options(error.to_string()).to_js_value())
}

/// An `ArrayBuffer`'s length, read as a number: `byteLength` can pass
/// `u32::MAX`, which the typed binding would wrap.
fn byte_length(buffer: &ArrayBuffer) -> u64 {
  js_sys::Reflect::get(buffer, &JsValue::from_str("byteLength"))
    .ok()
    .and_then(|length| length.as_f64())
    .map_or(u64::MAX, |length| length as u64)
}

/// The host's raw heightmap bytes, read a piece at a time straight from
/// its `ArrayBuffer`, so the input is never copied into WASM memory whole.
struct HostBytes(ArrayBuffer);

impl crate::dem::RawBytes for HostBytes {
  fn byte_length(&self) -> u64 {
    byte_length(&self.0)
  }

  fn read(&self, offset: usize, out: &mut [u8]) {
    Uint8Array::new_with_byte_offset_and_length(&self.0, offset as u32, out.len() as u32)
      .copy_to(out);
  }
}

fn to_js<T>(value: &T) -> Result<JsValue, JsValue>
where
  T: serde::Serialize,
{
  serde_wasm_bindgen::to_value(value)
    .map_err(|error| VistaError::internal(error.to_string()).to_js_value())
}
