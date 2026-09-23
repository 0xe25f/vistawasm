//! Weather patterns.
//!
//! The weather system blends between [`WeatherKind`] profiles over time and
//! produces one [`WeatherState`] per frame. The engine applies that state
//! to clouds, mist, wind, water, precipitation, ground wetness, snow cover,
//! and lightning, but only for the systems enabled in
//! [`WeatherOptions::effects`]; everything else keeps its manual settings.
//!
//! Everything here is a pure function of the options and elapsed time, so
//! a given seed always produces the same sequence of weather.

use vista_types::{WeatherKind, WeatherOptions, WeatherState};

use crate::maths::{hash_u64, smoothstep};

/// Target values for one weather state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherProfile {
  /// Cloud coverage.
  pub cloud_coverage: f32,
  /// Cloud density.
  pub cloud_density: f32,
  /// Multiplier on cloud layer thickness.
  pub cloud_thickness: f32,
  /// Ground mist density.
  pub mist_density: f32,
  /// Multiplier on haze distance (lower is hazier).
  pub haze: f32,
  /// Mean wind speed in metres per second.
  pub wind: f32,
  /// Gust strength relative to the mean wind.
  pub gustiness: f32,
  /// Rain intensity.
  pub rain: f32,
  /// Snowfall intensity.
  pub snow: f32,
  /// Lightning flashes per minute.
  pub lightning_per_minute: f32,
  /// Cloud type, 0 (cumulus) to 1 (flat sheet).
  pub stratiform: f32,
  /// Amount of towering storm clouds.
  pub towering: f32,
  /// Darkening of cloud bases.
  pub base_darkness: f32,
  /// Raggedness of cloud bases.
  pub ragged_base: f32,
  /// Visible rain or snow curtains below the clouds.
  pub rain_shafts: f32,
}

/// The profile for a weather state.
pub fn profile(kind: WeatherKind) -> WeatherProfile {
  let base = WeatherProfile {
    cloud_coverage: 0.0,
    cloud_density: 0.5,
    cloud_thickness: 1.0,
    mist_density: 0.0,
    haze: 1.0,
    wind: 3.0,
    gustiness: 0.3,
    rain: 0.0,
    snow: 0.0,
    lightning_per_minute: 0.0,
    stratiform: 0.0,
    towering: 0.0,
    base_darkness: 0.0,
    ragged_base: 0.0,
    rain_shafts: 0.0,
  };

  match kind {
    WeatherKind::Clear => WeatherProfile {
      cloud_coverage: 0.08,
      wind: 2.5,
      ..base
    },
    WeatherKind::PartlyCloudy => WeatherProfile {
      cloud_coverage: 0.45,
      cloud_density: 0.6,
      wind: 4.5,
      ..base
    },
    WeatherKind::Overcast => WeatherProfile {
      cloud_coverage: 0.92,
      cloud_density: 0.75,
      cloud_thickness: 1.2,
      haze: 0.6,
      wind: 6.0,
      stratiform: 0.85,
      base_darkness: 0.2,
      ..base
    },
    WeatherKind::Fog => WeatherProfile {
      cloud_coverage: 0.7,
      cloud_density: 0.4,
      cloud_thickness: 0.6,
      mist_density: 0.85,
      haze: 0.18,
      wind: 1.0,
      gustiness: 0.1,
      stratiform: 1.0,
      ..base
    },
    WeatherKind::Rain => WeatherProfile {
      cloud_coverage: 0.97,
      cloud_density: 0.9,
      cloud_thickness: 1.4,
      mist_density: 0.25,
      haze: 0.35,
      wind: 8.0,
      gustiness: 0.45,
      rain: 0.65,
      stratiform: 0.75,
      base_darkness: 0.6,
      ragged_base: 0.7,
      rain_shafts: 0.7,
      ..base
    },
    WeatherKind::Storm => WeatherProfile {
      // Storm cells with breaks between them, so the towers stand out.
      cloud_coverage: 0.82,
      cloud_density: 1.0,
      cloud_thickness: 1.8,
      mist_density: 0.3,
      haze: 0.25,
      wind: 17.0,
      gustiness: 0.7,
      // Above 1: the reported rain stays at full, and the extra makes the
      // downpour look heavier than ordinary rain.
      rain: 1.6,
      lightning_per_minute: 7.0,
      stratiform: 0.35,
      towering: 0.85,
      base_darkness: 0.85,
      ragged_base: 0.8,
      rain_shafts: 1.0,
      ..base
    },
    WeatherKind::Snow => WeatherProfile {
      cloud_coverage: 0.95,
      cloud_density: 0.8,
      cloud_thickness: 1.2,
      mist_density: 0.2,
      haze: 0.3,
      wind: 5.0,
      gustiness: 0.35,
      snow: 0.8,
      stratiform: 0.8,
      base_darkness: 0.3,
      ragged_base: 0.4,
      rain_shafts: 0.45,
      ..base
    },
  }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
  a + (b - a) * t
}

fn unit(value: u64) -> f32 {
  (value >> 40) as f32 / (1u64 << 24) as f32
}

/// The next state in an auto-cycling sequence: a small Markov chain that
/// favours plausible progressions (clear skies cloud over, overcast turns
/// to rain, storms ease back to rain).
pub fn next_kind(current: WeatherKind, roll: f32, allow_snow: bool) -> WeatherKind {
  use WeatherKind::*;

  let table: &[(WeatherKind, f32)] = match current {
    Clear => &[(PartlyCloudy, 0.75), (Fog, 0.15), (Overcast, 0.1)],
    PartlyCloudy => &[(Clear, 0.35), (Overcast, 0.45), (Fog, 0.1), (Rain, 0.1)],
    Overcast => &[(Rain, 0.45), (PartlyCloudy, 0.3), (Snow, 0.15), (Fog, 0.1)],
    Fog => &[(PartlyCloudy, 0.5), (Overcast, 0.4), (Clear, 0.1)],
    Rain => &[(Overcast, 0.45), (Storm, 0.3), (PartlyCloudy, 0.25)],
    Storm => &[(Rain, 0.7), (Overcast, 0.3)],
    Snow => &[(Overcast, 0.6), (PartlyCloudy, 0.4)],
  };
  let total: f32 = table
    .iter()
    .filter(|(kind, _)| allow_snow || *kind != Snow)
    .map(|(_, weight)| weight)
    .sum();
  let mut remaining = roll.clamp(0.0, 0.9999) * total;

  for (kind, weight) in table {
    if !allow_snow && *kind == Snow {
      continue;
    }

    if remaining < *weight {
      return *kind;
    }

    remaining -= weight;
  }

  PartlyCloudy
}

/// Runs the weather over time.
#[derive(Clone, Debug)]
pub struct WeatherSystem {
  options: WeatherOptions,
  from: WeatherKind,
  to: WeatherKind,
  /// Seconds since the current transition started.
  transition_elapsed: f32,
  /// Seconds the current state should last before cycling on.
  hold_seconds: f32,
  /// Number of cycled transitions so far (seeds the next roll).
  step: u64,
  time: f64,
  wetness: f32,
  snow_cover: f32,
  /// Precipitation beyond full intensity: 1 for ordinary rain or snow, up
  /// to about 3 for a storm with `precipitationScale` 2. Drives how heavy
  /// falling rain and snow look.
  heaviness: f32,
  next_lightning: f64,
  lightning_started: f64,
  lightning_offset: [f32; 2],
  state: WeatherState,
}

impl WeatherSystem {
  /// Start the weather in `options.state`, fully settled.
  pub fn new(options: WeatherOptions) -> Self {
    let start = options.state;
    let mut system = Self {
      from: start,
      to: start,
      transition_elapsed: f32::MAX,
      hold_seconds: 0.0,
      step: 0,
      time: 0.0,
      wetness: 0.0,
      snow_cover: 0.0,
      heaviness: 1.0,
      next_lightning: 4.0,
      lightning_started: -100.0,
      lightning_offset: [0.0, 4_000.0],
      state: WeatherState::default(),
      options,
    };
    system.hold_seconds = system.roll_hold();
    // Start with ground conditions already matching the weather.
    let settled = profile(start);
    system.wetness = settled.rain.min(1.0);
    system.snow_cover = settled.snow.min(1.0);
    system.advance(0.0);
    system
  }

  /// Current options.
  pub fn options(&self) -> &WeatherOptions {
    &self.options
  }

  /// Replace the options. Changing the target state starts a transition
  /// from the current blend instead of jumping.
  pub fn set_options(&mut self, options: WeatherOptions) {
    if options.state != self.options.state && options.state != self.to {
      self.begin_transition(options.state);
    }

    self.options = options;
  }

  /// The most recently computed state.
  pub fn state(&self) -> &WeatherState {
    &self.state
  }

  fn roll(&self, salt: u64) -> f32 {
    unit(hash_u64(
      self.options.seed_offset ^ self.step.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ salt,
    ))
  }

  fn roll_hold(&self) -> f32 {
    self.options.state_duration_seconds.max(1.0) * (0.6 + self.roll(0x51) * 0.8)
  }

  fn begin_transition(&mut self, target: WeatherKind) {
    // Freeze the current look as the new starting point.
    let blend = self.blend();
    self.from = if blend >= 0.5 { self.to } else { self.from };
    self.to = target;
    self.transition_elapsed = 0.0;
  }

  fn blend(&self) -> f32 {
    let duration = self.options.transition_seconds.max(0.001);
    smoothstep(self.transition_elapsed / duration)
  }

  /// Advance the simulation by `seconds` and return the new state.
  pub fn advance(&mut self, seconds: f32) -> &WeatherState {
    let dt = seconds.clamp(0.0, 5.0);
    self.time += dt as f64;
    self.transition_elapsed = (self.transition_elapsed + dt).min(f32::MAX / 2.0);

    if self.options.auto_cycle && self.blend() >= 1.0 {
      self.hold_seconds -= dt;

      if self.hold_seconds <= 0.0 {
        self.step += 1;
        let next = next_kind(self.to, self.roll(0x77), self.options.allow_snow);
        self.begin_transition(next);
        self.hold_seconds = self.roll_hold();
      }
    }

    let t = self.blend();
    let a = profile(self.from);
    let b = profile(self.to);
    let mix = |x: f32, y: f32| lerp(x, y, t);
    let scale = self.options.precipitation_scale.max(0.0);
    let raw = (mix(a.rain, b.rain) + mix(a.snow, b.snow)) * scale;
    let rain = (mix(a.rain, b.rain) * scale).min(1.0);
    let snow = (mix(a.snow, b.snow) * scale).min(1.0);
    self.heaviness = if rain + snow > 0.001 {
      (raw / (rain + snow)).max(1.0)
    } else {
      1.0
    };

    // Ground responds slowly: puddles take minutes to form and dry, snow
    // settles over a minute or two and melts more slowly than it falls.
    let wet_target = rain.max(snow * 0.3);
    let wet_rate = if wet_target > self.wetness {
      1.0 / 90.0
    } else {
      1.0 / 240.0
    };
    self.wetness += (wet_target - self.wetness) * (dt * wet_rate).min(1.0);
    let snow_rate = if snow > self.snow_cover {
      1.0 / 80.0
    } else {
      1.0 / 300.0
    };
    self.snow_cover += (snow - self.snow_cover) * (dt * snow_rate).min(1.0);

    // Gusts: two incommensurate slow waves give irregular gusting.
    let time = self.time as f32;
    let gust_wave = (time * 0.37).sin() * 0.6 + (time * 0.113 + 1.7).sin() * 0.4;
    let gustiness = mix(a.gustiness, b.gustiness);
    let wind =
      mix(a.wind, b.wind) * self.options.wind_scale.max(0.0) * (1.0 + gust_wave * gustiness * 0.5);

    // Lightning: random flashes at the blended rate, each a quick double
    // flicker.
    let rate = mix(a.lightning_per_minute, b.lightning_per_minute);
    let mut lightning = 0.0;

    if rate > 0.05 {
      if self.time >= self.next_lightning {
        self.lightning_started = self.time;
        self.step = self.step.wrapping_add(1);
        // Strike somewhere between one and nine kilometres away.
        let angle = self.roll(0x21) * std::f32::consts::TAU;
        let distance = 1_000.0 + self.roll(0x31) * 8_000.0;
        self.lightning_offset = [angle.sin() * distance, angle.cos() * distance];
        let wait = -((1.0 - self.roll(0x11) * 0.98).ln()) * 60.0 / rate;
        self.next_lightning = self.time + wait.clamp(1.5, 120.0) as f64;
      }

      let since = (self.time - self.lightning_started) as f32;

      if since < 0.6 {
        lightning = (1.0 - since / 0.6) * (0.6 + 0.4 * (since * 40.0).sin().abs());
      }
    }

    self.state = WeatherState {
      from: self.from,
      to: self.to,
      blend: t,
      cloud_coverage: mix(a.cloud_coverage, b.cloud_coverage),
      cloud_density: mix(a.cloud_density, b.cloud_density),
      mist_density: mix(a.mist_density, b.mist_density),
      wind_speed_metres_per_second: wind.max(0.0),
      wind_direction_degrees: self.options.wind_direction_degrees
        + (time * 0.05).sin() * 12.0 * gustiness,
      rain,
      snow,
      wetness: self.wetness.clamp(0.0, 1.0),
      snow_cover: self.snow_cover.clamp(0.0, 1.0),
      lightning: lightning.clamp(0.0, 1.0),
      stratiform: mix(a.stratiform, b.stratiform),
      towering: mix(a.towering, b.towering),
      base_darkness: mix(a.base_darkness, b.base_darkness),
      ragged_base: mix(a.ragged_base, b.ragged_base),
      rain_shafts: (mix(a.rain_shafts, b.rain_shafts) * scale).min(1.0),
    };
    &self.state
  }

  /// Horizontal offset, in metres from the camera, of the most recent
  /// lightning strike, so the clouds around it can light up.
  pub fn lightning_offset(&self) -> [f32; 2] {
    self.lightning_offset
  }

  /// Multiplier on cloud thickness for the current blend.
  pub fn cloud_thickness_scale(&self) -> f32 {
    lerp(
      profile(self.from).cloud_thickness,
      profile(self.to).cloud_thickness,
      self.blend(),
    )
  }

  /// Multiplier on haze distance for the current blend.
  pub fn haze_scale(&self) -> f32 {
    lerp(profile(self.from).haze, profile(self.to).haze, self.blend())
  }

  /// How far precipitation exceeds full intensity (1 or more).
  pub fn precipitation_heaviness(&self) -> f32 {
    self.heaviness
  }

  /// The state that currently dominates the blend.
  pub fn dominant(&self) -> WeatherKind {
    if self.blend() >= 0.5 {
      self.to
    } else {
      self.from
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn options(state: WeatherKind) -> WeatherOptions {
    WeatherOptions {
      enabled: true,
      state,
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

    let storm = profile(WeatherKind::Storm).cloud_coverage;

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
}
