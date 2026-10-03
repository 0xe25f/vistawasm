//! The weather preset table.
//!
//! Every kind of weather is a preset: a set of numbers (coverage, wind,
//! rain, humidity and so on), the presets that may follow it and how long
//! it lasts. The built-in presets are defined here; `WeatherOptions::presets`
//! overrides their fields or adds new presets, which may inherit from
//! another with `extends`. The weather system blends resolved presets
//! field by field, so every consumer reads one consistent state.

use vista_types::{NamedMap, ResolvedWeatherPreset, WeatherClimate, WeatherKind, WeatherPreset};

use crate::errors::{VistaError, VistaResult};

/// Number of blended numeric fields in a preset.
pub const FIELDS: usize = vista_types::WEATHER_PRESET_NUMBERS;

/// Indices of the blended fields in [`Values`].
pub mod field {
  pub const COVERAGE: usize = 0;
  pub const DENSITY: usize = 1;
  pub const THICKNESS: usize = 2;
  pub const STRATIFORM: usize = 3;
  pub const TOWERING: usize = 4;
  pub const BASE_DARKNESS: usize = 5;
  pub const RAGGED_BASE: usize = 6;
  pub const RAIN_SHAFTS: usize = 7;
  pub const CIRRUS: usize = 8;
  pub const HUMIDITY: usize = 9;
  pub const TURBIDITY: usize = 10;
  pub const HAZE: usize = 11;
  pub const MIST: usize = 12;
  pub const TEMPERATURE: usize = 13;
  pub const WIND: usize = 14;
  pub const GUSTINESS: usize = 15;
  pub const RAIN: usize = 16;
  pub const SNOW: usize = 17;
  pub const LIGHTNING: usize = 18;
  pub const COVERAGE_SPREAD: usize = 19;
  pub const PRECIPITATION_SPREAD: usize = 20;
  pub const CELL_SIZE: usize = 21;
  pub const CELLULARITY: usize = 22;
  pub const BASE_VARIATION: usize = 23;
  pub const BASE_LUMPINESS: usize = 24;
  pub const ALTOCUMULUS: usize = 25;
  pub const ALTOSTRATUS: usize = 26;
  pub const ALTO_HEIGHT: usize = 27;
  pub const ALTO_SPEED: usize = 28;
}

/// Each field's valid range, in [`WeatherPreset::NUMBER_NAMES`] order.
const RANGES: [(f32, f32); FIELDS] = [
  (0.0, 1.0),
  (0.0, 1.0),
  (0.2, 3.0),
  (0.0, 1.0),
  (0.0, 1.0),
  (0.0, 1.0),
  (0.0, 1.0),
  (0.0, 1.0),
  (0.0, 1.0),
  (0.0, 1.0),
  (2.0, 10.0),
  (0.05, 2.0),
  (0.0, 1.0),
  (-30.0, 30.0),
  (0.0, 60.0),
  (0.0, 1.0),
  (0.0, 2.0),
  (0.0, 2.0),
  (0.0, 60.0),
  (0.0, 1.0),
  (0.0, 1.0),
  (1.0, 100.0),
  (0.0, 1.0),
  (0.0, 0.2),
  (0.0, 1.0),
  (0.0, 1.0),
  (0.0, 1.0),
  (2_000.0, 7_000.0),
  (0.0, 4.0),
];

/// A preset's blended numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Values(pub [f32; FIELDS]);

impl Values {
  /// Linear blend from `self` (0) to `other` (1).
  pub fn lerp(&self, other: &Self, t: f32) -> Self {
    let mut out = self.0;

    for (value, target) in out.iter_mut().zip(other.0) {
      *value += (target - *value) * t;
    }

    Self(out)
  }

  /// A field.
  pub fn get(&self, index: usize) -> f32 {
    self.0[index]
  }

  /// Whether rain or snow falls.
  pub fn precipitates(&self) -> bool {
    self.0[field::RAIN] + self.0[field::SNOW] > 0.01
  }
}

/// A resolved preset: every field set.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
  /// Blended numbers.
  pub values: Values,
  /// Successor weights, by name.
  pub next: NamedMap<f32>,
  /// Shortest and longest time the preset lasts when cycling, or `None` to
  /// follow `WeatherOptions::state_duration_seconds`.
  pub durations: Option<(f32, f32)>,
  /// Temperatures the preset suits.
  pub climate: WeatherClimate,
}

/// The fields a preset leaves unset take these values.
const BASE: [f32; FIELDS] = [
  0.0, 0.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.35, 0.5, 3.0, 1.0, 0.0, 0.0, 3.0, 0.3, 0.0, 0.0, 0.0,
  0.15, 0.2, 12.0, 0.0, 0.07, 0.6, 0.0, 0.0, 4_200.0, 1.0,
];

/// Built-in presets: `(name, [(field, value)], [(successor, weight)], climate
/// max °C)`. The first seven keep the values of the seven weather states
/// before presets existed.
type BuiltIn = (
  &'static str,
  &'static [(usize, f32)],
  &'static [(&'static str, f32)],
  Option<f32>,
);

const BUILT_IN: [BuiltIn; 13] = {
  use field::*;
  [
    (
      "clear",
      &[
        (COVERAGE, 0.08),
        (WIND, 2.5),
        (HUMIDITY, 0.45),
        (COVERAGE_SPREAD, 0.08),
      ],
      &[
        ("fewClouds", 0.45),
        ("partlyCloudy", 0.3),
        ("fog", 0.1),
        ("mist", 0.05),
        ("overcast", 0.1),
      ],
      None,
    ),
    (
      "fewClouds",
      &[
        (COVERAGE, 0.2),
        (DENSITY, 0.55),
        (WIND, 3.5),
        (HUMIDITY, 0.5),
        (COVERAGE_SPREAD, 0.12),
        (ALTOCUMULUS, 0.15),
      ],
      &[
        ("clear", 0.35),
        ("partlyCloudy", 0.5),
        ("brokenClouds", 0.15),
      ],
      None,
    ),
    (
      "partlyCloudy",
      &[
        (COVERAGE, 0.45),
        (DENSITY, 0.6),
        (WIND, 4.5),
        (HUMIDITY, 0.55),
        (ALTOCUMULUS, 0.2),
      ],
      &[
        ("clear", 0.2),
        ("fewClouds", 0.15),
        ("brokenClouds", 0.2),
        ("overcast", 0.25),
        ("fog", 0.1),
        ("rain", 0.1),
      ],
      None,
    ),
    (
      "brokenClouds",
      &[
        (COVERAGE, 0.7),
        (DENSITY, 0.7),
        (THICKNESS, 1.1),
        (WIND, 5.5),
        (STRATIFORM, 0.3),
        (BASE_DARKNESS, 0.1),
        (HUMIDITY, 0.62),
        (COVERAGE_SPREAD, 0.25),
        (ALTOCUMULUS, 0.45),
        (ALTOSTRATUS, 0.1),
      ],
      &[
        ("partlyCloudy", 0.35),
        ("fewClouds", 0.15),
        ("overcast", 0.3),
        ("lightRain", 0.2),
      ],
      None,
    ),
    (
      "overcast",
      &[
        (COVERAGE, 0.92),
        (DENSITY, 0.75),
        (THICKNESS, 1.2),
        (HAZE, 0.6),
        (WIND, 6.0),
        (STRATIFORM, 0.85),
        (BASE_DARKNESS, 0.2),
        (HUMIDITY, 0.75),
        (TURBIDITY, 3.5),
        (COVERAGE_SPREAD, 0.1),
        (ALTOCUMULUS, 0.1),
        (ALTOSTRATUS, 0.65),
      ],
      &[
        ("rain", 0.35),
        ("lightRain", 0.1),
        ("brokenClouds", 0.15),
        ("partlyCloudy", 0.15),
        ("snow", 0.15),
        ("fog", 0.1),
      ],
      None,
    ),
    (
      "mist",
      &[
        (COVERAGE, 0.3),
        (DENSITY, 0.45),
        (THICKNESS, 0.8),
        (MIST, 0.35),
        (HAZE, 0.45),
        (WIND, 1.5),
        (GUSTINESS, 0.15),
        (STRATIFORM, 0.5),
        (HUMIDITY, 0.9),
        (TURBIDITY, 3.5),
        (ALTOSTRATUS, 0.3),
      ],
      &[
        ("clear", 0.3),
        ("fewClouds", 0.4),
        ("fog", 0.15),
        ("overcast", 0.15),
      ],
      None,
    ),
    (
      "fog",
      &[
        (COVERAGE, 0.7),
        (DENSITY, 0.4),
        (THICKNESS, 0.6),
        (MIST, 0.85),
        (HAZE, 0.18),
        (WIND, 1.0),
        (GUSTINESS, 0.1),
        (STRATIFORM, 1.0),
        (HUMIDITY, 1.0),
        (COVERAGE_SPREAD, 0.08),
        (ALTOSTRATUS, 0.3),
      ],
      &[
        ("partlyCloudy", 0.4),
        ("mist", 0.1),
        ("overcast", 0.4),
        ("clear", 0.1),
      ],
      None,
    ),
    (
      "lightRain",
      &[
        (COVERAGE, 0.9),
        (DENSITY, 0.8),
        (THICKNESS, 1.25),
        (MIST, 0.15),
        (HAZE, 0.5),
        (WIND, 6.0),
        (GUSTINESS, 0.35),
        (RAIN, 0.35),
        (STRATIFORM, 0.8),
        (BASE_DARKNESS, 0.4),
        (RAGGED_BASE, 0.5),
        (RAIN_SHAFTS, 0.45),
        (HUMIDITY, 0.88),
        (COVERAGE_SPREAD, 0.12),
        (ALTOSTRATUS, 0.8),
      ],
      &[("rain", 0.3), ("overcast", 0.35), ("brokenClouds", 0.35)],
      None,
    ),
    (
      "rain",
      &[
        (COVERAGE, 0.97),
        (DENSITY, 0.9),
        (THICKNESS, 1.4),
        (MIST, 0.25),
        (HAZE, 0.35),
        (WIND, 8.0),
        (GUSTINESS, 0.45),
        (RAIN, 0.65),
        (STRATIFORM, 0.75),
        (BASE_DARKNESS, 0.6),
        (RAGGED_BASE, 0.7),
        (RAIN_SHAFTS, 0.7),
        (HUMIDITY, 0.92),
        (COVERAGE_SPREAD, 0.12),
        (ALTOSTRATUS, 0.95),
      ],
      &[
        ("overcast", 0.35),
        ("storm", 0.2),
        ("heavyRain", 0.1),
        ("lightRain", 0.1),
        ("partlyCloudy", 0.1),
        ("brokenClouds", 0.15),
      ],
      None,
    ),
    (
      "heavyRain",
      &[
        (COVERAGE, 0.98),
        (DENSITY, 0.95),
        (THICKNESS, 1.5),
        (MIST, 0.3),
        (HAZE, 0.28),
        (WIND, 10.0),
        (GUSTINESS, 0.5),
        (RAIN, 1.3),
        (STRATIFORM, 0.7),
        (BASE_DARKNESS, 0.75),
        (RAGGED_BASE, 0.8),
        (RAIN_SHAFTS, 1.0),
        (HUMIDITY, 0.95),
        (COVERAGE_SPREAD, 0.1),
        (ALTOSTRATUS, 0.95),
      ],
      &[("rain", 0.6), ("storm", 0.25), ("overcast", 0.15)],
      None,
    ),
    (
      "storm",
      &[
        // Storm cells with breaks between them, so the towers stand out.
        (COVERAGE, 0.82),
        (DENSITY, 1.0),
        (THICKNESS, 1.8),
        (MIST, 0.3),
        (HAZE, 0.25),
        (WIND, 17.0),
        (GUSTINESS, 0.7),
        // Above 1: the reported rain stays at full, and the extra makes the
        // downpour look heavier than ordinary rain.
        (RAIN, 1.6),
        (LIGHTNING, 7.0),
        (STRATIFORM, 0.35),
        (TOWERING, 0.85),
        (BASE_DARKNESS, 0.85),
        (RAGGED_BASE, 0.8),
        (RAIN_SHAFTS, 1.0),
        (HUMIDITY, 0.9),
        (TURBIDITY, 3.5),
        (COVERAGE_SPREAD, 0.3),
        (PRECIPITATION_SPREAD, 0.3),
        (CELLULARITY, 0.8),
        (ALTOCUMULUS, 0.1),
        (ALTOSTRATUS, 0.5),
      ],
      &[("rain", 0.6), ("heavyRain", 0.1), ("overcast", 0.3)],
      None,
    ),
    (
      "snow",
      &[
        (COVERAGE, 0.95),
        (DENSITY, 0.8),
        (THICKNESS, 1.2),
        (MIST, 0.2),
        (HAZE, 0.3),
        (WIND, 5.0),
        (GUSTINESS, 0.35),
        (SNOW, 0.8),
        (STRATIFORM, 0.8),
        (BASE_DARKNESS, 0.3),
        (RAGGED_BASE, 0.4),
        (RAIN_SHAFTS, 0.45),
        (HUMIDITY, 0.85),
        (TURBIDITY, 2.5),
        (COVERAGE_SPREAD, 0.1),
        (ALTOSTRATUS, 0.85),
      ],
      &[
        ("overcast", 0.5),
        ("partlyCloudy", 0.3),
        ("brokenClouds", 0.1),
        ("blizzard", 0.1),
      ],
      Some(1.0),
    ),
    (
      "blizzard",
      &[
        (COVERAGE, 0.98),
        (DENSITY, 0.9),
        (THICKNESS, 1.4),
        (MIST, 0.35),
        (HAZE, 0.2),
        (WIND, 16.0),
        (GUSTINESS, 0.8),
        (SNOW, 1.4),
        (STRATIFORM, 0.85),
        (BASE_DARKNESS, 0.45),
        (RAGGED_BASE, 0.6),
        (RAIN_SHAFTS, 0.7),
        (HUMIDITY, 0.8),
        (TURBIDITY, 2.5),
        (COVERAGE_SPREAD, 0.08),
        (ALTOSTRATUS, 0.9),
      ],
      &[("snow", 0.7), ("overcast", 0.3)],
      Some(1.0),
    ),
  ]
};

/// Names of the built-in presets.
pub fn built_in_names() -> impl Iterator<Item = &'static str> {
  BUILT_IN.iter().map(|(name, ..)| *name)
}

fn built_in(name: &str) -> Option<Preset> {
  let (_, fields, next, max_celsius) = BUILT_IN.iter().find(|entry| entry.0 == name)?;
  let mut values = BASE;

  for (index, value) in fields.iter() {
    values[*index] = *value;
  }

  Some(Preset {
    values: Values(values),
    next: NamedMap(
      next
        .iter()
        .map(|(name, weight)| (name.to_string(), *weight))
        .collect(),
    ),
    durations: None,
    climate: WeatherClimate {
      min_celsius: None,
      max_celsius: *max_celsius,
    },
  })
}

/// Every resolved preset, by name.
#[derive(Clone, Debug, PartialEq)]
pub struct PresetTable {
  presets: NamedMap<Preset>,
}

impl Default for PresetTable {
  fn default() -> Self {
    Self {
      presets: NamedMap(
        built_in_names()
          .filter_map(|name| Some((name.to_string(), built_in(name)?)))
          .collect(),
      ),
    }
  }
}

impl PresetTable {
  /// The built-in presets with `custom` applied: overrides of built-in
  /// names change only the fields given, new names add presets, and
  /// `extends` chains are followed. Every error names the preset and field.
  pub fn resolve(custom: &NamedMap<WeatherPreset>) -> VistaResult<Self> {
    let mut table = Self::default();
    let mut done = NamedMap::default();

    for (name, _) in custom.iter() {
      resolve_one(name, custom, &mut done, &mut Vec::new())?;
    }

    for (name, preset) in done.0 {
      table.presets.insert(name, preset);
    }

    for (name, preset) in table.presets.iter() {
      for (successor, _) in preset.next.iter() {
        if table.presets.get(successor).is_none() {
          return Err(VistaError::options(format!(
            "weather.presets.{name}.next names `{successor}`, which is not a preset. Presets are: {}.",
            table.list()
          )));
        }
      }
    }

    Ok(table)
  }

  /// Every preset's name, in order, separated by commas, for messages.
  pub fn list(&self) -> String {
    let mut out = String::new();

    for name in self.names() {
      if !out.is_empty() {
        out.push_str(", ");
      }

      out.push_str(name);
    }

    out
  }

  /// A preset by name.
  pub fn get(&self, kind: &WeatherKind) -> Option<&Preset> {
    self.presets.get(kind.as_str())
  }

  /// A preset's values, or `partlyCloudy`'s when the name is unknown.
  pub fn values(&self, kind: &WeatherKind) -> Values {
    self
      .get(kind)
      .or_else(|| self.get(&WeatherKind::PartlyCloudy))
      .map_or(Values(BASE), |preset| preset.values)
  }

  /// Every preset's name, in order.
  pub fn names(&self) -> impl Iterator<Item = &str> {
    self.presets.iter().map(|(name, _)| name.as_str())
  }

  /// Every preset in its public form, durations defaulting to
  /// `state_duration_seconds`.
  pub fn public(&self, state_duration_seconds: f32) -> NamedMap<ResolvedWeatherPreset> {
    NamedMap(
      self
        .presets
        .iter()
        .map(|(name, preset)| {
          let (min, max) = preset
            .durations
            .unwrap_or((state_duration_seconds * 0.6, state_duration_seconds * 1.4));
          let resolved = ResolvedWeatherPreset {
            numbers: preset.values.0,
            next: preset.next.clone(),
            min_duration_seconds: min,
            max_duration_seconds: max,
            climate: preset.climate.clone(),
          };
          (name.clone(), resolved)
        })
        .collect(),
    )
  }
}

/// A preset's field must be a number from `min` to `max`.
fn check(name: &str, field_name: &str, value: f32, min: f32, max: f32) -> VistaResult<()> {
  if value.is_finite() && (min..=max).contains(&value) {
    return Ok(());
  }

  Err(VistaError::options(format!(
    "weather.presets.{name}.{field_name} must be between {min} and {max}, but it is {value}."
  )))
}

/// A preset's lower bound must not be above its upper bound.
fn ordered(name: &str, low_name: &str, high_name: &str, low: f32, high: f32) -> VistaResult<()> {
  if low <= high {
    return Ok(());
  }

  Err(VistaError::options(format!(
    "weather.presets.{name}.{low_name} ({low}) must not be above {high_name} ({high})."
  )))
}

fn resolve_one(
  name: &str,
  custom: &NamedMap<WeatherPreset>,
  done: &mut NamedMap<Preset>,
  chain: &mut Vec<String>,
) -> VistaResult<Preset> {
  if let Some(preset) = done.get(name) {
    return Ok(preset.clone());
  }

  let Some(given_preset) = custom.get(name) else {
    return built_in(name).ok_or_else(|| {
      VistaError::options(format!(
        "weather.presets.{} extends `{name}`, which is not a preset.",
        chain.last().map_or("", String::as_str)
      ))
    });
  };

  if chain.iter().any(|link| link == name) {
    let mut cycle = String::new();

    for link in chain.iter() {
      cycle.push_str(link);
      cycle.push_str(" → ");
    }

    cycle.push_str(name);
    return Err(VistaError::options(format!(
      "weather.presets: `extends` forms a cycle ({cycle}). Break it by removing one `extends`."
    )));
  }

  if name.is_empty() {
    return Err(VistaError::options(
      "weather.presets: a preset name must not be empty.",
    ));
  }

  chain.push(name.to_string());
  let mut preset = match &given_preset.extends {
    Some(parent) => resolve_one(parent.as_str(), custom, done, chain)?,
    None => built_in(name)
      .or_else(|| built_in("partlyCloudy"))
      .ok_or_else(|| VistaError::internal("the built-in presets are missing."))?,
  };
  chain.pop();

  for (index, value) in given_preset.numbers().into_iter().enumerate() {
    if let Some(value) = value {
      let (min, max) = RANGES[index];
      check(name, WeatherPreset::NUMBER_NAMES[index], value, min, max)?;
      preset.values.0[index] = value;
    }
  }

  if let Some(next) = &given_preset.next {
    for (successor, weight) in next.iter() {
      check(name, &format!("next.{successor}"), *weight, 0.0, 1.0e6)?;
    }

    preset.next = next.clone();
  }

  let (min, max) = preset.durations.unzip();
  let min = given_preset.min_duration_seconds.or(min);
  let max = given_preset.max_duration_seconds.or(max);

  for (field_name, value) in [("minDurationSeconds", min), ("maxDurationSeconds", max)] {
    if let Some(value) = value {
      check(name, field_name, value, 1.0, 86_400.0)?;
    }
  }

  if let (Some(min), Some(max)) = (given_preset.min_duration_seconds, max) {
    ordered(name, "minDurationSeconds", "maxDurationSeconds", min, max)?;
  }

  preset.durations = match (min, max) {
    (None, None) => None,
    (min, max) => {
      let min = min.or(max).unwrap_or(1.0);
      Some((min, max.unwrap_or(min).max(min)))
    }
  };

  if let Some(climate) = &given_preset.climate {
    for (field_name, value) in [
      ("climate.minCelsius", climate.min_celsius),
      ("climate.maxCelsius", climate.max_celsius),
    ] {
      if let Some(value) = value {
        check(name, field_name, value, -60.0, 60.0)?;
      }
    }

    if let (Some(min), Some(max)) = (climate.min_celsius, climate.max_celsius) {
      ordered(name, "climate.minCelsius", "climate.maxCelsius", min, max)?;
    }

    preset.climate = climate.clone();
  }

  done.insert(name.to_string(), preset.clone());
  Ok(preset)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn custom(entries: &[(&str, WeatherPreset)]) -> NamedMap<WeatherPreset> {
    entries
      .iter()
      .map(|(name, preset)| (name.to_string(), preset.clone()))
      .collect()
  }

  #[test]
  fn the_seven_original_states_keep_their_profiles() {
    use field::*;
    let table = PresetTable::default();
    // (coverage, density, thickness, mist, haze, wind, gustiness, rain,
    // snow, lightning, stratiform, towering, base darkness, ragged base,
    // rain shafts), as the seven weather states had them before presets.
    let old: [(&str, [f32; 15]); 7] = [
      (
        "clear",
        [
          0.08, 0.5, 1.0, 0.0, 1.0, 2.5, 0.3, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ],
      ),
      (
        "partlyCloudy",
        [
          0.45, 0.6, 1.0, 0.0, 1.0, 4.5, 0.3, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ],
      ),
      (
        "overcast",
        [
          0.92, 0.75, 1.2, 0.0, 0.6, 6.0, 0.3, 0.0, 0.0, 0.0, 0.85, 0.0, 0.2, 0.0, 0.0,
        ],
      ),
      (
        "fog",
        [
          0.7, 0.4, 0.6, 0.85, 0.18, 1.0, 0.1, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0,
        ],
      ),
      (
        "rain",
        [
          0.97, 0.9, 1.4, 0.25, 0.35, 8.0, 0.45, 0.65, 0.0, 0.0, 0.75, 0.0, 0.6, 0.7, 0.7,
        ],
      ),
      (
        "storm",
        [
          0.82, 1.0, 1.8, 0.3, 0.25, 17.0, 0.7, 1.6, 0.0, 7.0, 0.35, 0.85, 0.85, 0.8, 1.0,
        ],
      ),
      (
        "snow",
        [
          0.95, 0.8, 1.2, 0.2, 0.3, 5.0, 0.35, 0.0, 0.8, 0.0, 0.8, 0.0, 0.3, 0.4, 0.45,
        ],
      ),
    ];
    let order = [
      COVERAGE,
      DENSITY,
      THICKNESS,
      MIST,
      HAZE,
      WIND,
      GUSTINESS,
      RAIN,
      SNOW,
      LIGHTNING,
      STRATIFORM,
      TOWERING,
      BASE_DARKNESS,
      RAGGED_BASE,
      RAIN_SHAFTS,
    ];

    for (name, expected) in old {
      let values = table.values(&WeatherKind::new(name));

      for (index, value) in order.iter().zip(expected) {
        assert_eq!(values.get(*index), value, "{name} field {index}");
      }
    }
  }

  #[test]
  fn every_built_in_preset_is_valid_and_reachable() {
    let table = PresetTable::resolve(&NamedMap::default()).unwrap();
    assert_eq!(table.names().count(), 13);

    for name in table.names() {
      let reached = table
        .presets
        .iter()
        .any(|(_, preset)| preset.next.get(name).is_some_and(|weight| *weight > 0.0));
      assert!(reached, "{name} is never reached");
    }

    let fields = table.public(240.0);
    let field = |name: &str| fields.get(name).unwrap();
    assert!((field("fewClouds").number("cloudCoverage").unwrap() - 0.2).abs() < 1e-6);
    assert!((field("brokenClouds").number("coverageSpread").unwrap() - 0.25).abs() < 1e-6);
    assert_eq!(field("blizzard").climate.max_celsius, Some(1.0));
    assert_eq!(field("snow").climate.max_celsius, Some(1.0));
  }

  #[test]
  fn overriding_one_field_changes_only_that_field() {
    let table = PresetTable::resolve(&custom(&[(
      "rain",
      WeatherPreset {
        humidity: Some(0.4),
        ..Default::default()
      },
    )]))
    .unwrap();
    let before = PresetTable::default();
    let rain = WeatherKind::Rain;

    for index in 0..FIELDS {
      let expected = if index == field::HUMIDITY {
        0.4
      } else {
        before.values(&rain).get(index)
      };
      assert_eq!(table.values(&rain).get(index), expected, "field {index}");
    }

    assert_eq!(
      table.get(&rain).unwrap().next,
      before.get(&rain).unwrap().next
    );
  }

  #[test]
  fn extends_inherits_unset_fields() {
    let table = PresetTable::resolve(&custom(&[
      (
        "tropicalDownpour",
        WeatherPreset {
          extends: Some(WeatherKind::HeavyRain),
          turbidity: Some(7.0),
          ..Default::default()
        },
      ),
      (
        "monsoon",
        WeatherPreset {
          extends: Some(WeatherKind::new("tropicalDownpour")),
          wind_metres_per_second: Some(14.0),
          ..Default::default()
        },
      ),
    ]))
    .unwrap();
    let heavy = table.values(&WeatherKind::HeavyRain);
    let monsoon = table.values(&WeatherKind::new("monsoon"));

    assert_eq!(monsoon.get(field::TURBIDITY), 7.0);
    assert_eq!(monsoon.get(field::WIND), 14.0);
    assert_eq!(monsoon.get(field::RAIN), heavy.get(field::RAIN));
    assert_eq!(
      table.get(&WeatherKind::new("monsoon")).unwrap().next,
      table.get(&WeatherKind::HeavyRain).unwrap().next
    );
  }

  #[test]
  fn an_extends_cycle_is_rejected_and_named() {
    let link = |to: &str| WeatherPreset {
      extends: Some(WeatherKind::new(to)),
      ..Default::default()
    };
    let message = PresetTable::resolve(&custom(&[("a", link("b")), ("b", link("a"))]))
      .unwrap_err()
      .to_string();

    assert!(message.contains("a → b → a"), "{message}");
  }

  #[test]
  fn unknown_fields_are_rejected_with_the_valid_ones() {
    use serde::de::value::{Error, MapDeserializer};
    use serde::Deserialize;
    // Serde's own map deserialiser, as `serde-wasm-bindgen` presents a
    // JavaScript object.
    let parse = |entries: Vec<(&str, f32)>| {
      WeatherPreset::deserialize(MapDeserializer::<_, Error>::new(entries.into_iter()))
        .map_err(|error| error.to_string())
    };
    let error = parse(vec![("cloudCover", 0.5)]).unwrap_err();

    assert!(error.contains("cloudCover"), "{error}");
    assert!(error.contains("cloudCoverage"), "{error}");
    assert!(error.contains("turbidity"), "{error}");

    let preset = parse(vec![("turbidity", 7.0), ("windMetresPerSecond", 12.0)]).unwrap();
    assert_eq!(preset.turbidity, Some(7.0));
    assert_eq!(preset.wind_metres_per_second, Some(12.0));
    assert_eq!(preset.humidity, None);
  }

  #[test]
  fn out_of_range_values_are_rejected() {
    let cases = [
      WeatherPreset {
        cloud_coverage: Some(1.5),
        ..Default::default()
      },
      WeatherPreset {
        turbidity: Some(1.0),
        ..Default::default()
      },
      WeatherPreset {
        wind_metres_per_second: Some(f32::NAN),
        ..Default::default()
      },
      WeatherPreset {
        min_duration_seconds: Some(0.0),
        ..Default::default()
      },
      WeatherPreset {
        climate: Some(WeatherClimate {
          min_celsius: Some(5.0),
          max_celsius: Some(-5.0),
        }),
        ..Default::default()
      },
      WeatherPreset {
        next: Some([("clear".to_string(), -1.0)].into_iter().collect()),
        ..Default::default()
      },
      WeatherPreset {
        next: Some([("nowhere".to_string(), 1.0)].into_iter().collect()),
        ..Default::default()
      },
    ];

    for preset in cases {
      let error = PresetTable::resolve(&custom(&[("mine", preset.clone())]));
      assert!(error.is_err(), "{preset:?} was accepted");
      assert!(error.unwrap_err().to_string().contains("mine"));
    }
  }

  #[test]
  fn the_built_in_presets_carry_the_mid_level_clouds() {
    use field::*;
    let table = PresetTable::default();
    // (preset, altocumulus, altostratus).
    let expected = [
      ("clear", 0.0, 0.0),
      ("fewClouds", 0.15, 0.0),
      ("partlyCloudy", 0.2, 0.0),
      ("brokenClouds", 0.45, 0.1),
      ("overcast", 0.1, 0.65),
      ("mist", 0.0, 0.3),
      ("fog", 0.0, 0.3),
      ("lightRain", 0.0, 0.8),
      ("rain", 0.0, 0.95),
      ("heavyRain", 0.0, 0.95),
      ("storm", 0.1, 0.5),
      ("snow", 0.0, 0.85),
      ("blizzard", 0.0, 0.9),
    ];

    for (name, altocumulus, altostratus) in expected {
      let values = table.values(&WeatherKind::new(name));
      assert_eq!(values.get(ALTOCUMULUS), altocumulus, "{name} altocumulus");
      assert_eq!(values.get(ALTOSTRATUS), altostratus, "{name} altostratus");
      // Every preset keeps the defaults for the rest.
      assert_eq!(values.get(BASE_VARIATION), 0.07, "{name}");
      assert_eq!(values.get(BASE_LUMPINESS), 0.6, "{name}");
      assert_eq!(values.get(ALTO_HEIGHT), 4_200.0, "{name}");
      assert_eq!(values.get(ALTO_SPEED), 1.0, "{name}");
    }

    let public = table.public(240.0);
    let overcast = public.get("overcast").unwrap();
    assert_eq!(overcast.number("altostratus"), Some(0.65));
    assert_eq!(overcast.number("altoHeightMetres"), Some(4_200.0));
  }

  #[test]
  fn blending_halfway_gives_the_mean_of_the_new_fields() {
    use field::*;
    let table = PresetTable::resolve(&custom(&[(
      "brokenClouds",
      WeatherPreset {
        base_variation: Some(0.15),
        base_lumpiness: Some(0.2),
        alto_height_metres: Some(6_000.0),
        alto_speed: Some(3.0),
        ..Default::default()
      },
    )]))
    .unwrap();
    let a = table.values(&WeatherKind::Overcast);
    let b = table.values(&WeatherKind::BrokenClouds);
    let half = a.lerp(&b, 0.5);

    for index in [
      BASE_VARIATION,
      BASE_LUMPINESS,
      ALTOCUMULUS,
      ALTOSTRATUS,
      ALTO_HEIGHT,
      ALTO_SPEED,
    ] {
      let mean = (a.get(index) + b.get(index)) * 0.5;
      assert!((half.get(index) - mean).abs() < 1e-4, "field {index}");
    }

    assert!((half.get(ALTOSTRATUS) - 0.375).abs() < 1e-6);
    assert!((half.get(ALTO_HEIGHT) - 5_100.0).abs() < 1e-3);
  }

  #[test]
  fn out_of_range_cloud_base_and_alto_values_are_rejected() {
    // (field, a value below its range, a value above it).
    let cases: [(&str, f32, f32); 6] = [
      ("baseVariation", -0.01, 0.25),
      ("baseLumpiness", -0.1, 1.1),
      ("altocumulus", -0.1, 1.5),
      ("altostratus", -0.1, 1.5),
      ("altoHeightMetres", 1_500.0, 7_500.0),
      ("altoSpeed", -1.0, 5.0),
    ];

    use serde::de::value::{Error, MapDeserializer};
    use serde::Deserialize;
    let preset = |name: &str, value: f32| {
      WeatherPreset::deserialize(MapDeserializer::<_, Error>::new(
        vec![(name, value)].into_iter(),
      ))
      .unwrap()
    };

    for (field_name, low, high) in cases {
      let index = WeatherPreset::NUMBER_NAMES
        .iter()
        .position(|known| *known == field_name)
        .unwrap();

      for value in [low, high, f32::NAN] {
        let preset = preset(field_name, value);
        let error = PresetTable::resolve(&custom(&[("mine", preset)]))
          .unwrap_err()
          .to_string();
        assert!(error.contains(field_name), "{error}");
        assert!(error.contains("must be between"), "{error}");
      }

      let middle = preset(field_name, (RANGES[index].0 + RANGES[index].1) * 0.5);
      assert!(PresetTable::resolve(&custom(&[("mine", middle)])).is_ok());
    }
  }
}
