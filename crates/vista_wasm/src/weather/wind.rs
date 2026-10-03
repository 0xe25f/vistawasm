//! One wind for everything: the gust front that trees, grass, clouds,
//! rain and water share, and the sea state it raises.

use crate::maths::smoothstep;

/// Gust patterns are carried along at this share of the mean wind.
pub const GUST_TRAVEL: f32 = 1.0;

fn hash(value: f32) -> f32 {
  let mut x = (value * 0.1031).fract();
  x *= x + 33.33;
  (x * (x + x)).fract()
}

fn value_noise(s: f32) -> f32 {
  let cell = s.floor();
  let t = s - cell;
  let fade = t * t * (3.0 - 2.0 * t);
  let a = hash(cell);
  let b = hash(cell + 1.0);
  (a + (b - a) * fade) * 2.0 - 1.0
}

/// The gust front, -1 (a lull) to 1 (a gust), at `s` metres along the
/// wind from where the pattern started. The gust at a place is
/// `gust(dot(position, direction) - offset)`, with `offset` the distance
/// the pattern has travelled, so gusts sweep across the land downwind.
/// `common.wgsl`'s `gust_front` is the same function.
pub fn gust(s: f32) -> f32 {
  value_noise(s / 220.0) * 0.65 + value_noise(s / 70.0 + 17.0) * 0.35
}

/// The sea raised by the wind.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeaState {
  /// Significant wave height in metres.
  pub wave_height_metres: f32,
  /// Share of the sea under breaking whitecaps, 0 to 0.3.
  pub whitecaps: f32,
  /// Spray blown off the crests, 0 to 1.
  pub spray: f32,
}

/// Longest fetch the sea state counts, in metres.
pub const MAX_FETCH_METRES: f32 = 20_000.0;

/// The significant wave height the configured waves stand for: a light
/// breeze over the longest fetch.
pub fn reference_wave_height() -> f32 {
  sea_state(5.0, MAX_FETCH_METRES).wave_height_metres
}

/// The sea after wind of `wind` m/s has blown over `fetch_metres` of open
/// water: a fully developed sea of `0.0246 U^2` metres, or less when the
/// fetch limits it; whitecaps covering `3.84e-6 U^3.41` of the sea above
/// 4 m/s; and blown spray from 15 m/s.
pub fn sea_state(wind: f32, fetch_metres: f32) -> SeaState {
  let wind = wind.max(0.0);
  let fetch = fetch_metres.clamp(0.0, MAX_FETCH_METRES);
  let developed = 0.0246 * wind * wind;
  let limited = 0.0016 * wind * (fetch / 9.81).sqrt();
  let whitecaps = if wind < 4.0 {
    0.0
  } else {
    (3.84e-6 * wind.powf(3.41)).min(0.3)
  };

  SeaState {
    wave_height_metres: developed.min(limited),
    whitecaps,
    spray: smoothstep((wind - 15.0) / 8.0),
  }
}

/// Turn the waves' heading towards the wind over about a minute, the
/// short way round, in degrees.
pub fn turn_waves(current: f32, wind: f32, dt: f32) -> f32 {
  let difference = (wind - current + 540.0).rem_euclid(360.0) - 180.0;
  current + difference * (dt / 60.0).min(1.0)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn waves_grow_with_the_wind_and_whitecaps_need_four_metres_a_second() {
    let mut last = -1.0;

    for step in 0..=60 {
      let wind = step as f32 * 0.5;
      let sea = sea_state(wind, MAX_FETCH_METRES);
      assert!(
        sea.wave_height_metres > last || wind == 0.0,
        "at {wind} m/s"
      );
      last = sea.wave_height_metres;

      if wind < 4.0 {
        assert_eq!(sea.whitecaps, 0.0);
      }

      assert!(sea.whitecaps <= 0.3);
    }

    assert!(sea_state(10.0, MAX_FETCH_METRES).whitecaps > 0.005);
    assert_eq!(sea_state(10.0, 20_000.0).spray, 0.0);
    assert!(sea_state(24.0, 20_000.0).spray > 0.9);
    // A short fetch holds the sea down; a long one lets it build.
    assert!(
      sea_state(15.0, 2_000.0).wave_height_metres < sea_state(15.0, 20_000.0).wave_height_metres
    );
    // Light airs build a fully developed sea within the fetch.
    let calm = sea_state(2.0, 20_000.0).wave_height_metres;
    assert!((calm - 0.0246 * 4.0).abs() < 1e-5);
  }

  #[test]
  fn gusts_are_smooth_and_span_lulls_and_gusts() {
    let mut low: f32 = 1.0;
    let mut high: f32 = -1.0;

    for step in 0..4_000 {
      let s = step as f32 * 2.5;
      let g = gust(s);
      assert!((-1.0..=1.0).contains(&g));
      assert!((gust(s + 0.5) - g).abs() < 0.05);
      low = low.min(g);
      high = high.max(g);
    }

    assert!(low < -0.5 && high > 0.5);
  }

  #[test]
  fn waves_turn_to_the_wind_the_short_way() {
    let mut heading = 350.0;

    for _ in 0..240 {
      heading = turn_waves(heading, 20.0, 1.0);
    }

    assert!(
      ((heading - 20.0 + 540.0).rem_euclid(360.0) - 180.0).abs() < 1.0,
      "{heading}"
    );
    assert!(turn_waves(350.0, 20.0, 1.0) > 350.0);
  }
}
