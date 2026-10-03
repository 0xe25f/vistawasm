//! Frame pacing: a smoothed animation clock, and the controller that picks
//! the render scale needed to hold the frame rate.

/// A single frame interval longer than this (a background tab, a debugger
/// pause) is treated as a hiccup, not as time that passed in the scene.
const SPIKE_SECONDS: f32 = 0.2;
/// Longest interval counted when long frames keep coming, so a device that
/// is really that slow still animates and still lowers its resolution.
const LONGEST_SECONDS: f32 = 0.25;

/// Tells an isolated long interval from a run of slow frames.
#[derive(Clone, Debug, Default)]
struct SpikeFilter {
  long_run: u32,
}

impl SpikeFilter {
  /// The interval to use, or `None` for an isolated hiccup.
  fn filter(&mut self, interval: f32) -> Option<f32> {
    if interval <= 0.0 {
      return None;
    }

    if interval <= SPIKE_SECONDS {
      self.long_run = 0;
      return Some(interval);
    }

    self.long_run += 1;
    (self.long_run > 1).then_some(interval.min(LONGEST_SECONDS))
  }
}

/// Smooths the time step used for animation, so one late frame does not
/// make weather, wind, water, and clouds jump.
#[derive(Clone, Debug)]
pub struct FrameClock {
  smoothed: f32,
  spikes: SpikeFilter,
}

impl Default for FrameClock {
  fn default() -> Self {
    Self {
      smoothed: 1.0 / 60.0,
      spikes: SpikeFilter::default(),
    }
  }
}

impl FrameClock {
  /// Take the measured interval since the last frame and return the time
  /// step to animate by.
  pub fn step(&mut self, interval: f32) -> f32 {
    if let Some(interval) = self.spikes.filter(interval) {
      self.smoothed += (interval - self.smoothed) * 0.2;
    }

    self.smoothed
  }
}

/// How often the controller judges the frame rate, in seconds.
const WINDOW_SECONDS: f32 = 0.5;
/// Render scale steps, so the targets are not rebuilt for tiny changes.
const SCALE_STEP: f32 = 0.05;
/// Calm time needed before trying a higher scale, in seconds.
const CALM_SECONDS: f32 = 2.0;
/// How long a scale that dropped frames is not tried again, in seconds.
const CEILING_SECONDS: f32 = 10.0;

/// Chooses the render scale that holds the target frame rate. It watches
/// the real interval between rendered frames, which works in every browser:
/// when frames arrive late on average it lowers the scale in proportion,
/// and after a calm spell it tries one step higher, avoiding for a while a
/// scale that recently dropped frames.
#[derive(Clone, Debug)]
pub struct ResolutionController {
  scale: f32,
  window_time: f32,
  window_frames: u32,
  calm_time: f32,
  ceiling: f32,
  ceiling_time: f32,
  spikes: SpikeFilter,
  /// See [`Self::detail_pressure`].
  pressure: f32,
  /// See [`Self::shed_detail_first`].
  detail_first: bool,
}

impl Default for ResolutionController {
  fn default() -> Self {
    Self {
      scale: 1.0,
      window_time: 0.0,
      window_frames: 0,
      calm_time: 0.0,
      ceiling: 1.0,
      ceiling_time: 0.0,
      spikes: SpikeFilter::default(),
      pressure: 0.0,
      detail_first: false,
    }
  }
}

/// How much detail pressure rises or falls per judged window.
const PRESSURE_STEP: f32 = 0.25;
/// With [`ResolutionController::shed_detail_first`], the scale below
/// which resolution waits until detail pressure is at its most.
pub const DETAIL_SCALE: f32 = 0.85;

impl ResolutionController {
  /// Record one rendered frame and return the render scale to use.
  /// `frame_rate` is the rate to hold (the frame-rate cap, or 60 when
  /// uncapped); `range` is the highest and lowest scale allowed.
  pub fn update(&mut self, interval: f32, frame_rate: f32, range: (f32, f32)) -> f32 {
    let (max, min) = range;
    self.scale = self.scale.clamp(min, max);

    let Some(interval) = self.spikes.filter(interval) else {
      return self.scale;
    };

    if min >= max {
      return self.scale;
    }

    self.window_time += interval;
    self.window_frames += 1;

    if self.window_time < WINDOW_SECONDS {
      return self.scale;
    }

    let budget = 1.0 / if frame_rate > 0.0 { frame_rate } else { 60.0 };
    let average = self.window_time / self.window_frames as f32;
    let elapsed = self.window_time;
    self.window_time = 0.0;
    self.window_frames = 0;
    self.ceiling_time = (self.ceiling_time - elapsed).max(0.0);
    // Resolution may fall freely to `floor`; below it only once pressure
    // is at its most. Without detail to shed first, the floor is `min`.
    let floor = if self.detail_first {
      DETAIL_SCALE.clamp(min, max)
    } else {
      min
    };
    let at_floor = self.scale <= floor + 0.001;
    let late = average > budget * 1.12;
    let calm = average <= budget * 1.05;

    // Detail pressure asks for less scene detail once resolution alone
    // cannot hold the frame rate: it rises while frames stay late at the
    // floor, and falls in every calm window once the scale is back at it.
    if late && at_floor {
      self.pressure = (self.pressure + PRESSURE_STEP).min(1.0);
    } else if calm && self.scale >= floor - 0.001 {
      self.pressure = (self.pressure - PRESSURE_STEP).max(0.0);
    }

    if late && (!at_floor || self.pressure >= 1.0) {
      // Pixel count goes with the square of the scale.
      let factor = (budget / average).sqrt().clamp(0.75, 0.95);
      let lowered = ((self.scale * factor) / SCALE_STEP).floor() * SCALE_STEP;
      let lowest = if self.pressure >= 1.0 { min } else { floor };
      self.ceiling = self.scale;
      self.ceiling_time = CEILING_SECONDS;
      self.scale = lowered.clamp(lowest, max);
      self.calm_time = 0.0;
    } else if calm {
      self.calm_time += elapsed;

      // Above the floor, the scale waits for the pressure to clear.
      if self.calm_time >= CALM_SECONDS && (self.scale < floor - 0.001 || self.pressure <= 0.0) {
        self.calm_time = 0.0;
        let raised = (self.scale + SCALE_STEP).min(max);
        let raised = if self.scale < floor - 0.001 {
          raised.min(floor)
        } else {
          raised
        };

        if self.ceiling_time <= 0.0 || raised < self.ceiling - 0.001 {
          self.scale = raised;
        }
      }
    } else {
      self.calm_time = 0.0;
    }

    self.scale
  }

  /// How hard the scene should cut its detail, 0 to 1: 0 normally,
  /// rising by 0.25 for each judged window in which frames are still late
  /// with the scale at its minimum, and falling by 0.25 for each calm
  /// window. Vegetation shrinks its near radius by up to a half with it.
  pub fn detail_pressure(&self) -> f32 {
    self.pressure
  }

  /// Shed scene detail before resolution: with `on`, late frames lower
  /// the scale only to [`DETAIL_SCALE`], then raise detail pressure to
  /// its most, and only then lower the scale further. Recovery runs the
  /// other way. For scenes with streamed vegetation, whose near radius
  /// and full-mesh trees can give way first.
  pub fn shed_detail_first(&mut self, on: bool) {
    self.detail_first = on;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn run(controller: &mut ResolutionController, interval: f32, seconds: f32) -> f32 {
    let mut scale = 0.0;

    for _ in 0..(seconds / interval) as u32 {
      scale = controller.update(interval, 60.0, (1.0, 0.5));
    }

    scale
  }

  #[test]
  fn clock_smooths_and_ignores_spikes() {
    let mut clock = FrameClock::default();
    let steady = clock.step(1.0 / 60.0);
    assert!((steady - 1.0 / 60.0).abs() < 1e-6);
    // A two-second pause (a background tab) is not time in the scene.
    assert!((clock.step(2.0) - steady).abs() < 1e-6);
    // One late frame moves the step only part of the way.
    assert!(clock.step(0.05) < 0.03);

    // A device that really is that slow still animates.
    for _ in 0..40 {
      clock.step(2.0);
    }
    assert!(clock.step(2.0) > 0.24);
  }

  #[test]
  fn a_very_slow_device_still_lowers_its_scale() {
    let mut controller = ResolutionController::default();
    assert_eq!(run(&mut controller, 2.0, 30.0), 0.5);
  }

  #[test]
  fn one_long_pause_does_not_lower_the_scale() {
    let mut controller = ResolutionController::default();
    run(&mut controller, 1.0 / 60.0, 3.0);
    controller.update(3.0, 60.0, (1.0, 0.5));
    assert_eq!(run(&mut controller, 1.0 / 60.0, 3.0), 1.0);
  }

  #[test]
  fn lowers_the_scale_when_frames_are_late_and_holds_when_on_time() {
    let mut controller = ResolutionController::default();
    assert_eq!(run(&mut controller, 1.0 / 60.0, 3.0), 1.0);

    // Frames at 40 per second: well over budget, so the scale drops.
    let lowered = run(&mut controller, 1.0 / 40.0, 1.0);
    assert!((0.5..1.0).contains(&lowered));
    // Never below the minimum however slow.
    assert!(run(&mut controller, 1.0 / 10.0, 5.0) >= 0.5);
  }

  #[test]
  fn tries_higher_after_a_calm_spell_but_not_a_scale_that_just_failed() {
    let mut controller = ResolutionController::default();
    run(&mut controller, 1.0 / 40.0, 0.6);
    let lowered = controller.scale;

    // On time again: after the calm spell it steps up, but not back to
    // the scale that dropped frames until that is forgotten.
    let soon = run(&mut controller, 1.0 / 60.0, 5.0);
    assert!(soon > lowered && soon < 1.0);
    let later = run(&mut controller, 1.0 / 60.0, 20.0);
    assert!((later - 1.0).abs() < 1e-6);
  }

  #[test]
  fn detail_pressure_rises_only_at_the_lowest_scale_while_frames_are_late() {
    let mut controller = ResolutionController::default();

    // Late frames first lower the scale; pressure waits for the minimum.
    while controller.scale > 0.5 + 1e-3 {
      assert_eq!(controller.detail_pressure(), 0.0);
      run(&mut controller, 1.0 / 40.0, 0.5);
    }

    assert_eq!(controller.scale, 0.5);

    // Late windows at the minimum raise it by 0.25 each, up to 1.
    run(&mut controller, 1.0 / 40.0, 0.6);
    let pressure = controller.detail_pressure();
    assert!(pressure > 0.0 && pressure <= 0.5, "{pressure}");
    run(&mut controller, 1.0 / 40.0, 3.0);
    assert_eq!(controller.detail_pressure(), 1.0);

    // Calm windows take it down by 0.25 each.
    // (The first window may still hold late frames from before.)
    run(&mut controller, 1.0 / 60.0, 1.2);
    let pressure = controller.detail_pressure();
    assert!((0.25..=0.75).contains(&pressure), "{pressure}");
    run(&mut controller, 1.0 / 60.0, 3.0);
    assert_eq!(controller.detail_pressure(), 0.0);

    // With dynamic resolution off there is no pressure.
    let mut fixed = ResolutionController::default();

    for _ in 0..200 {
      fixed.update(1.0 / 20.0, 60.0, (0.75, 0.75));
    }

    assert_eq!(fixed.detail_pressure(), 0.0);
  }

  #[test]
  fn detail_gives_way_before_the_scale_falls_below_the_detail_scale() {
    let mut controller = ResolutionController::default();
    controller.shed_detail_first(true);
    let mut lowest_before_pressure = 1.0f32;

    // A long run of late frames: the scale stops at 0.85 while pressure
    // rises, and only falls further once pressure is at its most.
    for _ in 0..200 {
      let scale = controller.update(1.0 / 30.0, 60.0, (1.0, 0.5));

      if controller.detail_pressure() < 1.0 {
        lowest_before_pressure = lowest_before_pressure.min(scale);
      }
    }

    assert!(
      (lowest_before_pressure - DETAIL_SCALE).abs() < 1e-3,
      "{lowest_before_pressure}"
    );
    assert_eq!(controller.detail_pressure(), 1.0);
    assert!(controller.scale < DETAIL_SCALE);

    // On time again: the scale comes back to 0.85 before pressure falls,
    // and pressure clears before the scale rises above 0.85.
    let mut pressure_cleared_at = None;

    for step in 0..3000 {
      let scale = controller.update(1.0 / 60.0, 60.0, (1.0, 0.5));

      if controller.detail_pressure() < 1.0 {
        assert!(scale >= DETAIL_SCALE - 1e-3, "{scale}");
      }

      if scale > DETAIL_SCALE + 1e-3 {
        assert_eq!(controller.detail_pressure(), 0.0);
        pressure_cleared_at.get_or_insert(step);
      }
    }

    assert!(pressure_cleared_at.is_some());
    assert!((controller.scale - 1.0).abs() < 1e-6);
  }

  #[test]
  fn a_fixed_scale_is_left_alone() {
    let mut controller = ResolutionController::default();

    for _ in 0..200 {
      assert_eq!(controller.update(1.0 / 20.0, 60.0, (0.75, 0.75)), 0.75);
    }
  }
}
