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
    }
  }
}

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

    if average > budget * 1.12 {
      // Pixel count goes with the square of the scale.
      let factor = (budget / average).sqrt().clamp(0.75, 0.95);
      let lowered = ((self.scale * factor) / SCALE_STEP).floor() * SCALE_STEP;
      self.ceiling = self.scale;
      self.ceiling_time = CEILING_SECONDS;
      self.scale = lowered.clamp(min, max);
      self.calm_time = 0.0;
    } else if average <= budget * 1.05 {
      self.calm_time += elapsed;

      if self.calm_time >= CALM_SECONDS {
        self.calm_time = 0.0;
        let raised = (self.scale + SCALE_STEP).min(max);

        if self.ceiling_time <= 0.0 || raised < self.ceiling - 0.001 {
          self.scale = raised;
        }
      }
    } else {
      self.calm_time = 0.0;
    }

    self.scale
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
    assert!(lowered < 1.0 && lowered >= 0.5);
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
  fn a_fixed_scale_is_left_alone() {
    let mut controller = ResolutionController::default();

    for _ in 0..200 {
      assert_eq!(controller.update(1.0 / 20.0, 60.0, (0.75, 0.75)), 0.75);
    }
  }
}
