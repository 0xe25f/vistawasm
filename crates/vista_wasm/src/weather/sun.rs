//! The time of day and where it puts the sun, from the NOAA solar
//! position equations (the solar calculator's series for the sun's
//! longitude, obliquity and equation of time, and its refraction
//! correction).

use vista_types::{TimeOfDay, TimeOfDayOptions};

/// Where the sun is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolarPosition {
  /// Compass azimuth, degrees clockwise from north.
  pub azimuth_degrees: f32,
  /// Elevation above the horizon, with refraction, in degrees.
  pub elevation_degrees: f32,
}

struct Orbit {
  /// Equation of time in minutes.
  equation_minutes: f32,
  /// Declination in radians.
  declination: f32,
}

/// Days from the J2000 epoch to 1 January 2025, 00:00 UTC. The options
/// have no year, so the clock runs in 2025; other years move the sun by
/// under 0.1 degrees. Counting from J2000 keeps `f32` precise enough.
const JANUARY_2025: f32 = 9_131.5;

fn orbit(day_of_year: u32, hours: f32) -> Orbit {
  let t = (JANUARY_2025 + (day_of_year as f32 - 1.0) + hours / 24.0) / 36_525.0;
  let mean_longitude = (280.466_46 + t * (36_000.77 + t * 0.000_303_2)).rem_euclid(360.0);
  let anomaly = 357.5291 + t * (35_999.05 - 0.000_153_7 * t);
  let eccentricity = 0.016_708_634 - t * (0.000_042_037 + 0.000_000_126_7 * t);
  let m = anomaly.to_radians();
  let centre = m.sin() * (1.914_602 - t * (0.004_817 + 0.000_014 * t))
    + (2.0 * m).sin() * (0.019_993 - 0.000_101 * t)
    + (3.0 * m).sin() * 0.000_289;
  let omega = (125.04 - 1_934.136 * t).to_radians();
  let apparent_longitude =
    (mean_longitude + centre - 0.005_69 - 0.004_78 * omega.sin()).to_radians();
  let mean_obliquity =
    23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.000_59 - t * 0.001_813))) / 60.0) / 60.0;
  let obliquity = (mean_obliquity + 0.002_56 * omega.cos()).to_radians();
  let declination = (obliquity.sin() * apparent_longitude.sin()).asin();
  let y = (obliquity / 2.0).tan().powi(2);
  let l0 = mean_longitude.to_radians();
  let equation = y * (2.0 * l0).sin() - 2.0 * eccentricity * m.sin()
    + 4.0 * eccentricity * y * m.sin() * (2.0 * l0).cos()
    - 0.5 * y * y * (4.0 * l0).sin()
    - 1.25 * eccentricity * eccentricity * (2.0 * m).sin();
  Orbit {
    equation_minutes: 4.0 * equation.to_degrees(),
    declination,
  }
}

/// The sun at `hours` local time (the mean time of the time zone's own
/// meridian) on `day_of_year` at `latitude_degrees`.
pub fn solar_position(latitude_degrees: f32, day_of_year: u32, hours: f32) -> SolarPosition {
  let orbit = orbit(day_of_year, hours);
  let latitude = latitude_degrees.to_radians();
  let solar_minutes = hours * 60.0 + orbit.equation_minutes;
  let hour_angle = (solar_minutes / 4.0 - 180.0).to_radians();
  let cos_zenith = (latitude.sin() * orbit.declination.sin()
    + latitude.cos() * orbit.declination.cos() * hour_angle.cos())
  .clamp(-1.0, 1.0);
  let zenith = cos_zenith.acos();
  let azimuth = hour_angle
    .sin()
    .atan2(hour_angle.cos() * latitude.sin() - orbit.declination.tan() * latitude.cos())
    .to_degrees()
    + 180.0;
  let elevation = 90.0 - zenith.to_degrees();

  SolarPosition {
    azimuth_degrees: azimuth.rem_euclid(360.0),
    elevation_degrees: elevation + refraction(elevation),
  }
}

/// NOAA's atmospheric refraction correction, in degrees.
fn refraction(elevation: f32) -> f32 {
  let arc_seconds = if elevation > 85.0 {
    0.0
  } else if elevation > 5.0 {
    let t = elevation.to_radians().tan();
    58.1 / t - 0.07 / t.powi(3) + 0.000_086 / t.powi(5)
  } else if elevation > -0.575 {
    1_735.0 + elevation * (-518.2 + elevation * (103.4 + elevation * (-12.79 + elevation * 0.711)))
  } else {
    -20.772 / elevation.to_radians().tan()
  };
  arc_seconds / 3_600.0
}

/// Sunrise and sunset in local hours, or `None` in polar day or night.
pub fn sunrise_sunset(latitude_degrees: f32, day_of_year: u32) -> Option<(f32, f32)> {
  let orbit = orbit(day_of_year, 12.0);
  let latitude = latitude_degrees.to_radians();
  let cos_hour_angle = 90.833f32.to_radians().cos() / (latitude.cos() * orbit.declination.cos())
    - latitude.tan() * orbit.declination.tan();

  if !(-1.0..=1.0).contains(&cos_hour_angle) {
    return None;
  }

  let half_day_minutes = 4.0 * cos_hour_angle.acos().to_degrees();
  let noon = 720.0 - orbit.equation_minutes;
  Some((
    (noon - half_day_minutes) / 60.0,
    (noon + half_day_minutes) / 60.0,
  ))
}

/// A compass azimuth in the convention of `SunOptions::azimuth_degrees`,
/// where 0 points along +x and north is -z.
pub fn engine_azimuth(compass_degrees: f32) -> f32 {
  (compass_degrees - 90.0).rem_euclid(360.0)
}

/// The running clock.
#[derive(Clone, Debug, PartialEq)]
pub struct DayClock {
  options: TimeOfDayOptions,
  hours: f32,
}

impl DayClock {
  /// A clock set to `options.hours`.
  pub fn new(options: TimeOfDayOptions) -> Self {
    Self {
      hours: options.hours,
      options,
    }
  }

  /// Replace the options, setting the clock to `options.hours`.
  pub fn set_options(&mut self, options: TimeOfDayOptions) {
    *self = Self::new(options);
  }

  /// Whether the sun follows the clock.
  pub fn enabled(&self) -> bool {
    self.options.enabled
  }

  /// Local time, 0 to 24.
  pub fn hours(&self) -> f32 {
    self.hours
  }

  /// Game hours that pass per real second.
  pub fn hours_per_second(&self) -> f32 {
    24.0 / (self.options.day_length_minutes.max(1.0) * 60.0)
  }

  /// Run the clock on by `seconds` of real time, when enabled.
  pub fn advance(&mut self, seconds: f32) {
    if self.options.enabled {
      self.hours = (self.hours + seconds.max(0.0) * self.hours_per_second()).rem_euclid(24.0);
    }
  }

  /// Sunrise and sunset today.
  pub fn sunrise_sunset(&self) -> Option<(f32, f32)> {
    sunrise_sunset(self.options.latitude_degrees, self.options.day_of_year)
  }

  /// Where the sun is now.
  pub fn sun(&self) -> SolarPosition {
    solar_position(
      self.options.latitude_degrees,
      self.options.day_of_year,
      self.hours,
    )
  }

  /// The public report, with the azimuth in the engine's convention.
  pub fn report(&self) -> TimeOfDay {
    let sun = self.sun();
    let times = self.sunrise_sunset();
    TimeOfDay {
      hours: self.hours,
      sun_azimuth_degrees: engine_azimuth(sun.azimuth_degrees),
      sun_elevation_degrees: sun.elevation_degrees,
      sunrise_hours: times.map(|(rise, _)| rise),
      sunset_hours: times.map(|(_, set)| set),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Reference positions at longitude 0 (so local time is UTC) in 2025,
  /// from an independent ephemeris (PyEphem, 1010 hPa and 10 °C, with
  /// refraction):
  /// `(latitude, day of year, hours, compass azimuth, apparent elevation)`.
  const REFERENCE: [(f32, u32, f32, f32, f32); 6] = [
    (51.5, 172, 12.0, 179.09, 61.94),
    (51.5, 355, 9.0, 139.73, 5.65),
    (0.0, 80, 15.0, 270.72, 46.77),
    (-33.9, 1, 7.5, 98.95, 29.96),
    (64.1, 100, 18.5, 280.07, 4.46),
    (35.0, 250, 16.25, 259.57, 24.34),
  ];

  #[test]
  fn the_sun_is_where_an_ephemeris_puts_it() {
    for (latitude, day, hours, azimuth, elevation) in REFERENCE {
      let sun = solar_position(latitude, day, hours);
      let azimuth_error = ((sun.azimuth_degrees - azimuth + 540.0).rem_euclid(360.0) - 180.0).abs();
      assert!(
        azimuth_error < 0.5,
        "{latitude} {day} {hours}: azimuth {} against {azimuth}",
        sun.azimuth_degrees
      );
      assert!(
        (sun.elevation_degrees - elevation).abs() < 0.5,
        "{latitude} {day} {hours}: elevation {} against {elevation}",
        sun.elevation_degrees
      );
    }
  }

  #[test]
  fn polar_days_and_nights_have_no_sunrise() {
    assert!(sunrise_sunset(80.0, 172).is_none());
    assert!(sunrise_sunset(80.0, 355).is_none());
    let (rise, set) = sunrise_sunset(45.0, 172).unwrap();
    assert!(
      rise > 4.0 && rise < 5.0 && set > 19.0 && set < 20.0,
      "{rise} {set}"
    );
    // The sun sits on the horizon, refraction included, at sunset.
    let at_sunset = solar_position(45.0, 172, set);
    assert!(at_sunset.elevation_degrees.abs() < 0.6, "{at_sunset:?}");
  }

  #[test]
  fn the_clock_runs_with_real_time() {
    let mut clock = DayClock::new(TimeOfDayOptions {
      enabled: true,
      hours: 23.5,
      day_length_minutes: 24.0,
      ..Default::default()
    });
    // 24 real minutes per day: an hour a minute.
    clock.advance(60.0);
    assert!((clock.hours() - 0.5).abs() < 1e-4);
    assert_eq!(engine_azimuth(90.0), 0.0);
    assert_eq!(engine_azimuth(0.0), 270.0);
  }
}
