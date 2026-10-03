//! Weather patterns.
//!
//! The weather is one system built on a table of presets
//! ([`presets::PresetTable`]). The weather system blends the current
//! preset into the next one field by field, varies the blend across the
//! map ([`regional::RegionalField`]), and produces one [`WeatherState`]
//! per frame for the camera. The engine applies it to clouds, mist, wind,
//! water, precipitation, wet ground, snow, light and lightning, but only
//! for the systems enabled in [`WeatherOptions::effects`]; everything
//! else keeps its manual settings.
//!
//! Everything here is a pure function of the options and elapsed time, so
//! a given seed always produces the same sequence of weather.

pub mod presets;
pub mod regional;
pub mod sun;
pub mod surface;
pub mod wind;

use vista_types::{WeatherKind, WeatherOptions, WeatherState};

use crate::maths::{hash_u64, smoothstep};

use presets::{field, PresetTable, Values};
use regional::{FieldParams, RegionalField, RegionalSample};
use surface::{SurfaceCell, SurfaceInputs};

fn unit(value: u64) -> f32 {
  (value >> 40) as f32 / (1u64 << 24) as f32
}

/// Share of precipitation that falls as snow at a mean temperature in °C:
/// all of it below 0.5 °C, none above 2.5 °C, and sleet (a mix of rain and
/// snow) in between.
pub fn snow_fraction(celsius: f32) -> f32 {
  1.0 - smoothstep((celsius - 0.5) / 2.0)
}

/// The next state in an auto-cycling sequence among the built-in presets:
/// a small Markov chain that favours plausible progressions (clear skies
/// cloud over, overcast turns to rain, storms ease back to rain).
pub fn next_kind(current: WeatherKind, roll: f32, allow_snow: bool) -> WeatherKind {
  next_kind_in_climate(current, roll, allow_snow, false)
}

/// [`next_kind`], for a climate that may be below freezing. In the cold,
/// rain and storms come as snow, clear spells are half as likely again,
/// and snow is always allowed.
pub fn next_kind_in_climate(
  current: WeatherKind,
  roll: f32,
  allow_snow: bool,
  freezing: bool,
) -> WeatherKind {
  let celsius = freezing.then_some(-5.0);
  choose_next(
    &PresetTable::default(),
    &current,
    roll,
    allow_snow,
    celsius,
    false,
  )
}

/// The candidates that may follow `current`, with their weights: its
/// `next` table, in the cold with rain turned to snow (a blizzard in a
/// gale) and clear spells half as likely again, without presets whose
/// climate does not suit `celsius`, and without snowy presets unless snow
/// is allowed or it is freezing.
fn candidates(
  table: &PresetTable,
  current: &WeatherKind,
  allow_snow: bool,
  celsius: Option<f32>,
) -> Vec<(WeatherKind, f32)> {
  let freezing = celsius.is_some_and(|celsius| celsius < 0.0);
  let allow_snow = allow_snow || freezing;
  let mut out: Vec<(WeatherKind, f32)> = Vec::new();
  let Some(preset) = table.get(current) else {
    return out;
  };

  for (name, weight) in preset.next.iter() {
    let mut kind = WeatherKind::new(name.as_str());
    let mut weight = *weight;
    let Some(next) = table.get(&kind) else {
      continue;
    };
    let values = next.values;

    if freezing && values.get(field::RAIN) > 0.0 && values.get(field::SNOW) == 0.0 {
      kind = if values.get(field::WIND) >= 12.0 {
        WeatherKind::Blizzard
      } else {
        WeatherKind::Snow
      };
    } else if freezing && kind == WeatherKind::Clear {
      weight *= 1.5;
    }

    let Some(chosen) = table.get(&kind) else {
      continue;
    };
    let climate = &chosen.climate;
    let suits = celsius.is_none_or(|celsius| {
      climate.min_celsius.is_none_or(|min| celsius >= min)
        && climate.max_celsius.is_none_or(|max| celsius <= max)
    });

    if !suits || (!allow_snow && chosen.values.get(field::SNOW) > 0.0) {
      continue;
    }

    match out.iter_mut().find(|(existing, _)| *existing == kind) {
      Some(entry) => entry.1 += weight,
      None => out.push((kind, weight)),
    }
  }

  out
}

fn pick(candidates: &[(WeatherKind, f32)], roll: f32) -> Option<WeatherKind> {
  let total: f32 = candidates.iter().map(|(_, weight)| weight).sum();

  if total <= 0.0 {
    return None;
  }

  let mut remaining = roll.clamp(0.0, 0.9999) * total;

  for (kind, weight) in candidates {
    if remaining < *weight {
      return Some(kind.clone());
    }

    remaining -= weight;
  }

  candidates.last().map(|(kind, _)| kind.clone())
}

/// Choose the preset after `current` with a roll from 0 to 1. With
/// `settle`, a precipitating choice is swapped for a dry one (the evening
/// calm).
fn choose_next(
  table: &PresetTable,
  current: &WeatherKind,
  roll: f32,
  allow_snow: bool,
  celsius: Option<f32>,
  settle: bool,
) -> WeatherKind {
  let all = candidates(table, current, allow_snow, celsius);
  let chosen = pick(&all, roll);

  if settle
    && chosen
      .as_ref()
      .is_some_and(|kind| table.values(kind).precipitates())
  {
    let dry: Vec<_> = all
      .iter()
      .filter(|(kind, _)| !table.values(kind).precipitates())
      .cloned()
      .collect();

    if let Some(kind) = pick(&dry, roll) {
      return kind;
    }
  }

  chosen.unwrap_or(WeatherKind::PartlyCloudy)
}

/// The step after `current` on the way to a clear evening: storm to rain,
/// rain to broken cloud, broken cloud to a few clouds (in the cold,
/// through snow). `None` once there.
fn clearing_step(table: &PresetTable, current: &WeatherKind, cold: bool) -> Option<WeatherKind> {
  let heavy = |_: ()| {
    if cold {
      WeatherKind::Snow
    } else {
      WeatherKind::Rain
    }
  };

  let next = match current.as_str() {
    "storm" | "heavyRain" => heavy(()),
    "blizzard" => WeatherKind::Snow,
    "rain" | "lightRain" | "snow" | "overcast" => WeatherKind::BrokenClouds,
    "brokenClouds" => WeatherKind::FewClouds,
    _ if table.values(current).precipitates() => WeatherKind::BrokenClouds,
    _ => return None,
  };
  table.get(&next).map(|_| next)
}

/// How many steps the clearing chain takes from `current`.
fn clearing_steps(table: &PresetTable, current: &WeatherKind, cold: bool) -> u32 {
  let mut steps = 0;
  let mut kind = current.clone();

  while let Some(next) = clearing_step(table, &kind, cold) {
    steps += 1;
    kind = next;

    if steps > 8 {
      break;
    }
  }

  steps
}

/// Slope in degrees and hollow depth in metres (the mean of the four
/// neighbouring samples less the height, positive in hollows) at the
/// sample nearest a world position.
pub fn surface_relief(terrain: &crate::terrain::HeightMap, x: f32, z: f32) -> (f32, f32) {
  let metadata = &terrain.metadata;
  let metres = metadata.metres_per_sample.max(0.001);

  if metadata.width < 3 || metadata.height < 3 {
    return (0.0, 0.0);
  }

  let column = (x / metres + (metadata.width as f32 - 1.0) * 0.5)
    .round()
    .clamp(1.0, metadata.width as f32 - 2.0) as u32;
  let row = (z / metres + (metadata.height as f32 - 1.0) * 0.5)
    .round()
    .clamp(1.0, metadata.height as f32 - 2.0) as u32;
  let at = |x: u32, y: u32| terrain.height_at(x, y).unwrap_or(0.0);
  let (west, east) = (at(column - 1, row), at(column + 1, row));
  let (north, south) = (at(column, row - 1), at(column, row + 1));
  let gradient = ((east - west).powi(2) + (south - north).powi(2)).sqrt() / (2.0 * metres);
  let hollow = (west + east + north + south) * 0.25 - at(column, row);
  (gradient.atan().to_degrees(), hollow)
}

/// The time of day, for the golden-hour bias.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DayInfo {
  /// Local time, 0 to 24.
  pub hours: f32,
  /// Sunset, or `None` in polar day or night.
  pub sunset: Option<f32>,
  /// Game hours per real second.
  pub hours_per_second: f32,
}

/// Where the sun and the cloud layer are, for the sunlight the clouds let
/// through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sky {
  /// Unit vector towards the sun.
  pub sun: [f32; 3],
  /// Cloud base in metres.
  pub cloud_base: f32,
  /// Cloud layer thickness in metres.
  pub cloud_thickness: f32,
}

impl Default for Sky {
  fn default() -> Self {
    Self {
      sun: crate::maths::sun_direction_vector(132.0, 18.0),
      cloud_base: 1_400.0,
      cloud_thickness: 1_700.0,
    }
  }
}

/// Runs the weather over time.
#[derive(Clone, Debug)]
pub struct WeatherSystem {
  options: WeatherOptions,
  table: PresetTable,
  from: WeatherKind,
  to: WeatherKind,
  /// Seconds since the current transition started.
  transition_elapsed: f32,
  /// Length of the current transition.
  transition_seconds: f32,
  /// Seconds the current state should last before cycling on.
  hold_seconds: f32,
  /// Number of cycled transitions so far (seeds the next roll).
  step: u64,
  time: f64,
  /// The blended preset.
  values: Values,
  regional: RegionalField,
  /// Where the weather is seen.
  camera: [f32; 3],
  /// The weather at the camera.
  local: RegionalSample,
  /// Flat, open ground at the camera, for when there is no terrain.
  ground: SurfaceCell,
  /// Distance the gust pattern has travelled downwind, in metres.
  gust_offset: f32,
  sky: Sky,
  sun_transmittance: f32,
  /// Precipitation beyond full intensity: 1 for ordinary rain or snow, up
  /// to about 3 for a storm with `precipitationScale` 2. Drives how heavy
  /// falling rain and snow look.
  heaviness: f32,
  next_lightning: f64,
  lightning_started: f64,
  lightning_offset: [f32; 2],
  /// Mean temperature in °C where the weather is seen, when known.
  celsius: Option<f32>,
  day: Option<DayInfo>,
  /// Following the clearing chain towards a clear evening.
  clearing: bool,
  /// The golden-hour bias; switched off only by tests, for comparison.
  golden_hour: bool,
  /// Whether each step refreshes a band of the regional grid.
  refresh_grid: bool,
  /// Whether the next step refreshes the whole grid: the weather changed
  /// too fast for a band a frame to keep up.
  regrid: bool,
  state: WeatherState,
}

impl WeatherSystem {
  /// Start the weather in `options.state`, fully settled. Options are
  /// validated before they get here; presets that fail to resolve fall
  /// back to the built-in table.
  pub fn new(options: WeatherOptions) -> Self {
    let table = PresetTable::resolve(&options.presets).unwrap_or_default();
    let start = options.state.clone();
    let values = table.values(&start);
    let params = FieldParams::of(&values, options.precipitation_scale, options.regional);
    let regional = RegionalField::new(
      options.seed_offset,
      options.region_size_km * 1_000.0,
      params,
    );
    let mut system = Self {
      from: start.clone(),
      to: start,
      transition_elapsed: f32::MAX,
      transition_seconds: options.transition_seconds,
      hold_seconds: 0.0,
      step: 0,
      time: 0.0,
      values,
      regional,
      camera: [0.0; 3],
      local: RegionalSample::default(),
      ground: SurfaceCell::default(),
      gust_offset: 0.0,
      sky: Sky::default(),
      sun_transmittance: 1.0,
      heaviness: 1.0,
      next_lightning: 4.0,
      lightning_started: -100.0,
      lightning_offset: [0.0, 4_000.0],
      celsius: None,
      day: None,
      clearing: false,
      golden_hour: true,
      refresh_grid: true,
      regrid: false,
      state: WeatherState::default(),
      table,
      options,
    };
    system.hold_seconds = system.roll_hold();
    // Start with ground conditions already matching the weather.
    system.ground = system.settled_ground();
    system.advance(0.0);
    system
  }

  /// Ground conditions that match the current weather at the camera, as
  /// if it had lasted a while.
  pub fn settled_ground(&self) -> SurfaceCell {
    let scale = self.options.precipitation_scale.max(0.0);
    SurfaceCell {
      wetness: (self.values.get(field::RAIN) * scale).min(1.0),
      puddles: (self.values.get(field::RAIN) * scale - 0.6).clamp(0.0, 1.0),
      snow: (self.values.get(field::SNOW) * scale).min(1.0),
    }
  }

  /// Current options.
  pub fn options(&self) -> &WeatherOptions {
    &self.options
  }

  /// The resolved preset table.
  pub fn table(&self) -> &PresetTable {
    &self.table
  }

  /// Replace the options. Changing the target state starts a transition
  /// from the current blend instead of jumping.
  pub fn set_options(&mut self, options: WeatherOptions) {
    self.table = PresetTable::resolve(&options.presets).unwrap_or_default();

    let restart = options.state != self.options.state && options.state != self.to;

    if options.seed_offset != self.options.seed_offset {
      self.regional = RegionalField::new(
        options.seed_offset,
        options.region_size_km * 1_000.0,
        *self.regional.params(),
      );
    }

    self.regional.set_size(options.region_size_km * 1_000.0);
    self.options = options;

    if restart {
      self.begin_transition(self.options.state.clone(), self.options.transition_seconds);
    }
  }

  /// Set the mean temperature in °C where the weather is seen (under the
  /// camera), or `None` when it is unknown. It decides whether rain falls
  /// as rain, sleet, or snow, and biases the cycle towards cold weather.
  pub fn set_celsius(&mut self, celsius: Option<f32>) {
    self.celsius = celsius.filter(|value| value.is_finite());
  }

  /// Set where the weather is seen.
  pub fn set_camera(&mut self, position: [f32; 3]) {
    self.camera = position;
  }

  /// Set the sun and cloud layer, for the sunlight the clouds let through.
  pub fn set_sky(&mut self, sky: Sky) {
    self.sky = sky;
  }

  /// Set the time of day, or `None` when it does not run. With auto-cycle,
  /// the weather then tends to clear before sunset.
  pub fn set_day(&mut self, day: Option<DayInfo>) {
    self.day = day;
  }

  /// Switch the golden-hour bias off, to measure what it does.
  #[cfg(test)]
  pub fn set_golden_hour_bias(&mut self, on: bool) {
    self.golden_hour = on;
  }

  /// Whether each step refreshes a band of the regional grid: off while
  /// skipping ahead, which refreshes it once at the end.
  pub fn set_refresh_grid(&mut self, on: bool) {
    self.refresh_grid = on;
  }

  /// Refresh the grid rows over a terrain with half extents `half`.
  pub fn refresh_terrain_rows(&mut self, half: [f32; 2]) {
    if self.options.regional {
      self.regional.refresh_band(-half[1], half[1]);
    }
  }

  /// Refresh the whole grid.
  pub fn refresh_all_rows(&mut self) {
    if self.options.regional {
      self.regional.refresh_rows(regional::GRID);
    }
  }

  /// Report the ground under the camera from the surface map, in place of
  /// the flat, open ground the system assumes without a terrain.
  pub fn report_ground(&mut self, cell: SurfaceCell, capacity: f32) {
    self.state.wetness = cell.wetness;
    self.state.puddles = cell.puddles * capacity;
    self.state.snow_cover = cell.snow;
  }

  /// The temperature at the camera with the weather's offset, if known.
  pub fn air_celsius(&self) -> Option<f32> {
    self
      .celsius
      .map(|celsius| celsius + self.values.get(field::TEMPERATURE))
  }

  /// The most recently computed state.
  pub fn state(&self) -> &WeatherState {
    &self.state
  }

  /// The blended preset.
  pub fn values(&self) -> &Values {
    &self.values
  }

  /// The weather at the camera.
  pub fn local(&self) -> RegionalSample {
    self.local
  }

  /// The regional field.
  pub fn regional(&self) -> &RegionalField {
    &self.regional
  }

  /// The regional field, to take its grid for upload.
  pub fn regional_mut(&mut self) -> &mut RegionalField {
    &mut self.regional
  }

  /// Flat, open ground at the camera.
  pub fn camera_ground(&self) -> SurfaceCell {
    self.ground
  }

  /// Distance the gust pattern has travelled downwind, in metres.
  pub fn gust_offset(&self) -> f32 {
    self.gust_offset
  }

  /// Share of direct sunlight reaching the camera through the clouds.
  pub fn sun_transmittance(&self) -> f32 {
    self.sun_transmittance
  }

  /// Mean wind in m/s (before gusts), with `windScale`.
  pub fn mean_wind(&self) -> f32 {
    self.values.get(field::WIND) * self.options.wind_scale.max(0.0)
  }

  /// Whether the weather is, or may become, rain.
  pub fn rain_possible(&self) -> bool {
    self.options.auto_cycle
      || self.table.values(&self.to).get(field::RAIN) > 0.0
      || self.table.values(&self.from).get(field::RAIN) > 0.0
  }

  fn roll(&self, salt: u64) -> f32 {
    unit(hash_u64(
      self.options.seed_offset ^ self.step.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ salt,
    ))
  }

  fn roll_hold(&self) -> f32 {
    let default = self.options.state_duration_seconds.max(1.0);
    let (min, max) = self
      .table
      .get(&self.to)
      .and_then(|preset| preset.durations)
      .unwrap_or((default * 0.6, default * 1.4));
    min + (max - min) * self.roll(0x51)
  }

  fn begin_transition(&mut self, target: WeatherKind, seconds: f32) {
    // Freeze the current look as the new starting point.
    self.from = if self.blend() >= 0.5 {
      self.to.clone()
    } else {
      self.from.clone()
    };
    self.to = target;
    self.transition_elapsed = 0.0;
    self.transition_seconds = seconds;
  }

  fn blend(&self) -> f32 {
    let duration = self.transition_seconds.max(0.001);
    smoothstep(self.transition_elapsed / duration)
  }

  /// Hours until sunset, when the time of day runs and the sun sets.
  fn until_sunset(&self) -> Option<(f32, DayInfo)> {
    let day = self.day.filter(|_| self.golden_hour)?;
    let sunset = day.sunset?;
    Some(((sunset - day.hours).rem_euclid(24.0), day))
  }

  fn cycle(&mut self, dt: f32) {
    let cold = self.air_celsius().is_some_and(|celsius| celsius < 0.0);
    let window = self.until_sunset();

    // Between four and one and a half hours before sunset, rain or snow
    // starts a clearing: a chain of steps that ends in a few clouds by
    // about an hour before sunset. Once started, it runs to its end.
    match window {
      Some((until, _)) if (1.5..=4.0).contains(&until) => {
        if self.table.values(&self.to).precipitates() {
          self.clearing = true;
        }
      }
      Some((until, _)) if until > 0.0 && until < 1.5 => {}
      _ => self.clearing = false,
    }

    // Each step's share of the time left until an hour before sunset.
    let clearing = window.filter(|_| self.clearing).map(|(until, day)| {
      let steps = clearing_steps(&self.table, &self.to, cold).max(1);
      let budget = (until - 1.0).max(0.1) / day.hours_per_second.max(1e-6);
      budget / steps as f32
    });

    if let Some(per_step) = clearing {
      if self.blend() >= 1.0 {
        self.hold_seconds = self.hold_seconds.min(per_step * 0.6).max(0.0);
      }
    }

    if self.blend() < 1.0 {
      return;
    }

    self.hold_seconds -= dt;

    if self.hold_seconds > 0.0 {
      return;
    }

    self.step += 1;
    let mut transition = self.options.transition_seconds;
    let next = match clearing {
      Some(per_step) => {
        transition = transition.min(per_step * 0.4);
        clearing_step(&self.table, &self.to, cold)
      }
      None => None,
    };
    let next = next.unwrap_or_else(|| {
      // In the last hour and a half before sunset, rain and snow are 70 %
      // less likely to start.
      let settle = window.is_some_and(|(until, _)| until < 1.5) && self.roll(0x99) < 0.7;
      choose_next(
        &self.table,
        &self.to,
        self.roll(0x77),
        self.options.allow_snow,
        self.air_celsius(),
        settle,
      )
    });

    if clearing_step(&self.table, &next, cold).is_none() {
      self.clearing = false;
    }

    self.begin_transition(next, transition);
    self.hold_seconds = self.roll_hold();
  }

  /// Advance the simulation by `seconds` and return the new state.
  pub fn advance(&mut self, seconds: f32) -> &WeatherState {
    let dt = seconds.clamp(0.0, 5.0);
    self.time += dt as f64;
    self.transition_elapsed = (self.transition_elapsed + dt).min(f32::MAX / 2.0);

    if self.options.auto_cycle {
      self.cycle(dt);
    }

    let t = self.blend();
    let a = self.table.values(&self.from);
    let b = self.table.values(&self.to);
    self.values = a.lerp(&b, t);
    let values = self.values;
    let scale = self.options.precipitation_scale.max(0.0);

    // Wind: the mean, with gusts sweeping downwind across the land. The
    // clouds ride on winds aloft about twice as strong, and carry the
    // regional weather with them.
    let time = self.time as f32;
    let gustiness = values.get(field::GUSTINESS);
    let mean_wind = self.mean_wind();
    let direction = self.options.wind_direction_degrees + (time * 0.05).sin() * 12.0 * gustiness;
    let radians = direction.to_radians();
    let heading = [radians.sin(), radians.cos()];
    self.gust_offset = (self.gust_offset + mean_wind * wind::GUST_TRAVEL * dt) % 1.0e6;
    let along = self.camera[0] * heading[0] + self.camera[2] * heading[1];
    let gust = wind::gust(along - self.gust_offset);
    let wind = mean_wind * (1.0 + gust * gustiness * 0.5);
    // As fast as the clouds drift (`CloudsOptions::speed`), so the cloud
    // detail keeps its place in the weather.
    let aloft = wind * 2.0 * dt;
    let params = FieldParams::of(&values, scale, self.options.regional);

    // A band a frame follows a gradual transition; a sudden change (a
    // transition shorter than a few frames, or new presets) refreshes it
    // all.
    if params.jumps_from(self.regional.params()) {
      self.regrid = true;
    }

    self
      .regional
      .advance([heading[0] * aloft, heading[1] * aloft], params);

    if self.options.regional && self.refresh_grid {
      let rows = if std::mem::take(&mut self.regrid) {
        regional::GRID
      } else {
        regional::ROWS_PER_FRAME
      };
      self.regional.refresh_rows(rows);
    }

    self.local = self.regional.evaluate(self.camera[0], self.camera[2]);
    let local = self.local;
    self.sun_transmittance = if self.options.regional {
      self.regional.sun_transmittance(
        self.camera,
        self.sky.sun,
        self.sky.cloud_base,
        self.sky.cloud_thickness,
      )
    } else {
      regional::transmittance_of_coverage(local.coverage)
    };

    // The camera's precipitation, split between rain and snow as the
    // preset has them.
    let (preset_rain, preset_snow) = (values.get(field::RAIN), values.get(field::SNOW));
    let share = if preset_rain + preset_snow > 0.0 {
      preset_rain / (preset_rain + preset_snow)
    } else {
      1.0
    };
    let raw = local.precipitation;
    let mut rain = (raw * share).min(1.0);
    let mut snow = (raw * (1.0 - share)).min(1.0);

    // In the cold, rain falls as sleet or snow instead.
    if let Some(celsius) = self.air_celsius() {
      let frozen = snow_fraction(celsius);
      snow = (snow + rain * frozen).min(1.0);
      rain *= 1.0 - frozen;
    }

    self.heaviness = if rain + snow > 0.001 {
      (raw / (rain + snow)).max(1.0)
    } else {
      1.0
    };

    let sunlight = (self.sky.sun[1].max(0.0) * self.sun_transmittance).min(1.0);
    self.ground = surface::step(
      self.ground,
      &SurfaceInputs {
        precipitation: raw,
        celsius: self.air_celsius().unwrap_or(15.0),
        sun: sunlight,
        wind,
        patch: 0.5,
        ..Default::default()
      },
      dt,
    );

    let lightning = self.lightning(values.get(field::LIGHTNING));

    self.state = WeatherState {
      from: self.from.clone(),
      to: self.to.clone(),
      blend: t,
      cloud_coverage: local.coverage,
      cloud_density: values.get(field::DENSITY),
      mist_density: values.get(field::MIST),
      wind_speed_metres_per_second: wind.max(0.0),
      wind_direction_degrees: direction,
      rain,
      snow,
      wetness: self.ground.wetness,
      snow_cover: self.ground.snow,
      lightning,
      stratiform: values.get(field::STRATIFORM),
      towering: values.get(field::TOWERING),
      base_darkness: values.get(field::BASE_DARKNESS),
      ragged_base: values.get(field::RAGGED_BASE),
      rain_shafts: (values.get(field::RAIN_SHAFTS) * scale).min(1.0),
      humidity: local.humidity,
      turbidity: values.get(field::TURBIDITY),
      storminess: local.storminess,
      sun_transmittance: self.sun_transmittance,
      gustiness,
      puddles: self.ground.puddles,
    };
    &self.state
  }

  /// Lightning: random flashes at the blended rate, each a quick double
  /// flicker, struck where the rain is heaviest among a few spots one to
  /// nine kilometres away, and only where it rains.
  fn lightning(&mut self, rate: f32) -> f32 {
    if rate <= 0.05 {
      return 0.0;
    }

    if self.time >= self.next_lightning {
      self.step = self.step.wrapping_add(1);
      let mut best = (0.0, [0.0, 4_000.0]);

      for spot in 0..6u64 {
        let angle = self.roll(0x21 + spot * 2) * std::f32::consts::TAU;
        let distance = 1_000.0 + self.roll(0x31 + spot * 2) * 8_000.0;
        let offset = [angle.sin() * distance, angle.cos() * distance];
        let sample = self
          .regional
          .evaluate(self.camera[0] + offset[0], self.camera[2] + offset[1]);
        let score = sample.precipitation * (0.5 + sample.storminess);

        if score > best.0 {
          best = (score, offset);
        }
      }

      if best.0 > 0.15 {
        self.lightning_started = self.time;
        self.lightning_offset = best.1;
      }

      let wait = -((1.0 - self.roll(0x11) * 0.98).ln()) * 60.0 / rate;
      self.next_lightning = self.time + wait.clamp(1.5, 120.0) as f64;
    }

    let since = (self.time - self.lightning_started) as f32;

    if since < 0.6 {
      ((1.0 - since / 0.6) * (0.6 + 0.4 * (since * 40.0).sin().abs())).clamp(0.0, 1.0)
    } else {
      0.0
    }
  }

  /// Horizontal offset, in metres from the camera, of the most recent
  /// lightning strike, so the clouds around it can light up.
  pub fn lightning_offset(&self) -> [f32; 2] {
    self.lightning_offset
  }

  /// Multiplier on cloud thickness for the current blend.
  pub fn cloud_thickness_scale(&self) -> f32 {
    self.values.get(field::THICKNESS)
  }

  /// Multiplier on haze distance for the current blend: the preset's own
  /// haze, thinned by clean air and thickened by turbid, damp air. At the
  /// neutral turbidity of 3 and humidity of 0.5 it is the preset's haze.
  pub fn haze_scale(&self) -> f32 {
    self.values.get(field::HAZE) * 3.0 / self.values.get(field::TURBIDITY).max(1.0)
      * (1.15 - 0.3 * self.local.humidity)
  }

  /// How far precipitation exceeds full intensity (1 or more).
  pub fn precipitation_heaviness(&self) -> f32 {
    self.heaviness
  }

  /// The state that currently dominates the blend.
  pub fn dominant(&self) -> WeatherKind {
    if self.blend() >= 0.5 {
      self.to.clone()
    } else {
      self.from.clone()
    }
  }

  /// Seconds of weather time so far.
  pub fn time(&self) -> f64 {
    self.time
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  // These tests are about blending, cycling and precipitation, so the
  // weather is the same everywhere; `regional.rs` tests the variation.
  fn options(state: WeatherKind) -> WeatherOptions {
    WeatherOptions {
      enabled: true,
      state,
      regional: false,
      ..WeatherOptions::default()
    }
  }

  #[test]
  fn precipitation_beyond_full_intensity_counts_as_heaviness() {
    let rain = WeatherSystem::new(options(WeatherKind::Rain));
    assert!((rain.precipitation_heaviness() - 1.0).abs() < 1e-6);

    let storm = WeatherSystem::new(options(WeatherKind::Storm));
    assert!((storm.state().rain - 1.0).abs() < 1e-6);
    assert!(storm.precipitation_heaviness() > 1.5);

    let downpour = WeatherSystem::new(WeatherOptions {
      precipitation_scale: 2.0,
      ..options(WeatherKind::Rain)
    });
    assert!((downpour.state().rain - 1.0).abs() < 1e-6);
    assert!(downpour.precipitation_heaviness() > 1.2);
  }

  #[test]
  fn starts_settled_in_the_requested_state() {
    let system = WeatherSystem::new(options(WeatherKind::Rain));

    assert_eq!(system.state().to, WeatherKind::Rain);
    assert!(system.state().rain > 0.5);
    assert!(system.state().cloud_coverage > 0.9);
  }

  #[test]
  fn changing_state_blends_over_the_transition() {
    let mut system = WeatherSystem::new(options(WeatherKind::Clear));
    system.set_options(WeatherOptions {
      transition_seconds: 10.0,
      ..options(WeatherKind::Storm)
    });
    let early = system.advance(1.0).cloud_coverage;
    let mut late = early;

    for _ in 0..20 {
      late = system.advance(1.0).cloud_coverage;
    }

    let storm = PresetTable::default()
      .values(&WeatherKind::Storm)
      .get(field::COVERAGE);

    assert!(early < 0.5);
    assert!((late - storm).abs() < 0.01);
    assert_eq!(system.dominant(), WeatherKind::Storm);
  }

  #[test]
  fn ground_gets_wet_gradually_and_dries_slowly() {
    let mut system = WeatherSystem::new(options(WeatherKind::Clear));
    system.set_options(WeatherOptions {
      transition_seconds: 1.0,
      ..options(WeatherKind::Rain)
    });

    for _ in 0..30 {
      system.advance(1.0);
    }

    let wet = system.state().wetness;
    assert!(wet > 0.1 && wet < 0.6, "wetness {wet}");

    system.set_options(WeatherOptions {
      transition_seconds: 1.0,
      ..options(WeatherKind::Clear)
    });

    for _ in 0..30 {
      system.advance(1.0);
    }

    assert!(system.state().wetness > wet * 0.6);
  }

  #[test]
  fn auto_cycle_is_deterministic_for_a_seed() {
    let run = || {
      let mut system = WeatherSystem::new(WeatherOptions {
        auto_cycle: true,
        state_duration_seconds: 5.0,
        transition_seconds: 2.0,
        ..options(WeatherKind::Clear)
      });
      let mut kinds = Vec::new();

      for _ in 0..400 {
        system.advance(0.5);
        kinds.push(system.dominant());
      }

      kinds
    };

    let first = run();
    assert_eq!(first, run());
    assert!(first.iter().any(|kind| *kind != WeatherKind::Clear));
  }

  #[test]
  fn snow_only_occurs_when_allowed() {
    for roll in 0..100 {
      assert_ne!(
        next_kind(WeatherKind::Overcast, roll as f32 / 100.0, false),
        WeatherKind::Snow
      );
    }
  }

  #[test]
  fn rain_falls_as_snow_in_the_cold_and_as_sleet_near_freezing() {
    let mut cold = WeatherSystem::new(options(WeatherKind::Rain));
    cold.set_celsius(Some(-3.0));
    let state = cold.advance(0.1).clone();
    assert_eq!(state.rain, 0.0);
    assert!(state.snow > 0.5);
    assert_eq!(state.to, WeatherKind::Rain);

    let mut sleet = WeatherSystem::new(options(WeatherKind::Rain));
    sleet.set_celsius(Some(1.5));
    let state = sleet.advance(0.1).clone();
    assert!(state.rain > 0.2 && state.snow > 0.2, "{state:?}");

    let mut mild = WeatherSystem::new(options(WeatherKind::Rain));
    mild.set_celsius(Some(12.0));
    assert_eq!(mild.advance(0.1).snow, 0.0);

    // Snow settles on the ground in the cold.
    for _ in 0..200 {
      cold.advance(1.0);
    }

    assert!(cold.state().snow_cover > 0.5);
  }

  #[test]
  fn cold_climates_turn_rain_and_storms_into_snow_and_always_allow_it() {
    let freezing = |current, roll| next_kind_in_climate(current, roll, false, true);
    let mut clear = 0;
    let mut mild_clear = 0;

    for roll in 0..200 {
      let roll = roll as f32 / 200.0;
      let rainy = |kind: WeatherKind| kind == WeatherKind::Rain || kind == WeatherKind::Storm;
      assert!(!rainy(freezing(WeatherKind::PartlyCloudy, roll)));
      assert!(!rainy(freezing(WeatherKind::Overcast, roll)));
      assert!(!rainy(freezing(WeatherKind::Rain, roll)));
      clear += (freezing(WeatherKind::PartlyCloudy, roll) == WeatherKind::Clear) as u32;
      mild_clear +=
        (next_kind(WeatherKind::PartlyCloudy, roll, false) == WeatherKind::Clear) as u32;
    }

    assert!(clear > mild_clear);
    assert!((0..100)
      .any(|roll| freezing(WeatherKind::Overcast, roll as f32 / 100.0) == WeatherKind::Snow));
    assert_eq!(freezing(WeatherKind::Storm, 0.1), WeatherKind::Snow);
  }

  #[test]
  fn storms_flash_with_lightning() {
    let mut system = WeatherSystem::new(options(WeatherKind::Storm));
    let mut flashes = 0;

    for _ in 0..3_000 {
      if system.advance(0.05).lightning > 0.5 {
        flashes += 1;
      }
    }

    assert!(flashes > 0);
  }

  #[test]
  fn each_state_has_its_own_cloud_type() {
    let rain = WeatherSystem::new(options(WeatherKind::Rain))
      .state()
      .clone();
    let storm = WeatherSystem::new(options(WeatherKind::Storm))
      .state()
      .clone();
    let fair = WeatherSystem::new(options(WeatherKind::PartlyCloudy))
      .state()
      .clone();

    assert!(rain.stratiform > 0.5 && rain.rain_shafts > 0.5);
    assert!(storm.towering > 0.5 && storm.base_darkness > rain.base_darkness);
    assert_eq!(fair.towering, 0.0);
    assert_eq!(fair.rain_shafts, 0.0);
  }

  #[test]
  fn halfway_through_a_transition_every_field_is_the_mean() {
    let table = PresetTable::default();
    let mut system = WeatherSystem::new(options(WeatherKind::Clear));
    system.set_options(WeatherOptions {
      transition_seconds: 10.0,
      ..options(WeatherKind::Storm)
    });
    // Half of the smoothstep's span is half the blend.
    system.advance(5.0);
    let clear = table.values(&WeatherKind::Clear);
    let storm = table.values(&WeatherKind::Storm);

    for index in 0..presets::FIELDS {
      let mean = (clear.get(index) + storm.get(index)) * 0.5;
      assert!(
        (system.values().get(index) - mean).abs() < 1e-4,
        "field {index}: {} against {mean}",
        system.values().get(index)
      );
    }

    // And every consumer reads the blend.
    let state = system.state();
    assert!(
      (state.cloud_density - (clear.get(field::DENSITY) + storm.get(field::DENSITY)) * 0.5).abs()
        < 1e-4
    );
    assert!((state.towering - storm.get(field::TOWERING) * 0.5).abs() < 1e-4);
    assert!((system.cloud_thickness_scale() - (1.0 + 1.8) * 0.5).abs() < 1e-4);
  }

  #[test]
  fn custom_presets_can_be_the_state() {
    let mut presets = vista_types::NamedMap::default();
    presets.insert(
      "tropicalDownpour".to_string(),
      vista_types::WeatherPreset {
        extends: Some(WeatherKind::HeavyRain),
        humidity: Some(1.0),
        turbidity: Some(8.0),
        ..Default::default()
      },
    );
    let system = WeatherSystem::new(WeatherOptions {
      presets,
      ..options(WeatherKind::new("tropicalDownpour"))
    });

    assert_eq!(system.dominant().as_str(), "tropicalDownpour");
    assert!((system.state().turbidity - 8.0).abs() < 1e-6);
    assert!(system.state().rain > 0.9);
  }

  #[test]
  fn the_camera_reads_its_own_part_of_a_regional_storm() {
    let mut system = WeatherSystem::new(WeatherOptions {
      regional: true,
      ..options(WeatherKind::Storm)
    });
    // A high sun, so the light comes through the camera's own part of the
    // sky rather than a neighbouring cell's.
    system.set_sky(Sky {
      sun: crate::maths::sun_direction_vector(132.0, 80.0),
      ..Sky::default()
    });
    let (mut dry, mut wet) = (Vec::new(), Vec::new());

    for step in 0..400 {
      let position = [step as f32 * 211.0 - 40_000.0, 100.0, step as f32 * 37.0];
      system.set_camera(position);
      system.advance(0.0);
      let state = system.state();

      if state.cloud_coverage < 0.55 {
        assert_eq!(state.rain, 0.0);
        dry.push(state.sun_transmittance);
      } else if state.rain > 0.8 {
        wet.push(state.sun_transmittance);
      }
    }

    let mean = |values: &[f32]| values.iter().sum::<f32>() / values.len() as f32;
    assert!(!dry.is_empty() && !wet.is_empty());
    assert!(
      mean(&dry) > mean(&wet),
      "dry {}, wet {}",
      mean(&dry),
      mean(&wet)
    );
  }

  #[test]
  fn the_same_seed_and_time_give_the_same_regional_weather() {
    let run = || {
      let mut system = WeatherSystem::new(WeatherOptions {
        regional: true,
        auto_cycle: true,
        state_duration_seconds: 20.0,
        transition_seconds: 5.0,
        ..options(WeatherKind::Rain)
      });
      system.set_camera([1_500.0, 50.0, -800.0]);

      for _ in 0..300 {
        system.advance(0.25);
      }

      (system.state().clone(), system.regional().drift())
    };

    assert_eq!(run(), run());
  }

  /// Clear evenings in `days` days of a seeded, auto-cycling weather with
  /// the time of day: skies under half covered with nothing falling half
  /// an hour before sunset.
  fn clear_evenings(days: u32, bias: bool) -> u32 {
    let mut clock = sun::DayClock::new(vista_types::TimeOfDayOptions {
      enabled: true,
      hours: 6.0,
      ..Default::default()
    });
    let mut system = WeatherSystem::new(WeatherOptions {
      auto_cycle: true,
      ..options(WeatherKind::PartlyCloudy)
    });
    system.set_golden_hour_bias(bias);
    let (_, sunset) = clock.sunrise_sunset().unwrap();
    let check = sunset - 0.5;
    let mut clear = 0;
    let mut day = 0;
    let dt = 2.0;

    while day < days {
      let before = clock.hours();
      clock.advance(dt);
      system.set_day(Some(DayInfo {
        hours: clock.hours(),
        sunset: Some(sunset),
        hours_per_second: clock.hours_per_second(),
      }));
      let state = system.advance(dt);

      if before < check && clock.hours() >= check {
        day += 1;

        if state.cloud_coverage < 0.5 && state.rain + state.snow == 0.0 {
          clear += 1;
        }
      }
    }

    clear
  }

  #[test]
  fn the_weather_tends_to_clear_before_sunset() {
    let with_bias = clear_evenings(200, true);
    let without = clear_evenings(200, false);

    assert!(
      with_bias as f32 >= without as f32 * 1.3,
      "{with_bias} clear evenings with the bias, {without} without"
    );
    // Deterministic from the seed.
    assert_eq!(clear_evenings(30, true), clear_evenings(30, true));
  }
}
