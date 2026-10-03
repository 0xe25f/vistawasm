/** Frame-rate cap used when `RenderQualityOptions.maxFrameRate` is unset. */
export const DEFAULT_MAX_FRAME_RATE = 60;

const TOLERANCE_MS = 1.5;

/**
 * Decides which animation frames to render under a frame-rate cap. The
 * browser calls back at the display's refresh rate (60, 120, 144 Hz, and
 * so on); rendering only some of those callbacks, evenly spaced, gives a
 * steady frame rate instead of one that swings with the scene.
 */
export class FramePacer {
  private lastRenderMs: number | null = null;

  public constructor(private maxFrameRate = DEFAULT_MAX_FRAME_RATE) {}

  /** Set the cap in frames per second; 0 renders every animation frame. */
  public setMaxFrameRate(maxFrameRate: number): void {
    this.maxFrameRate = maxFrameRate;
  }

  /** Whether the animation frame at `nowMs` should be rendered. */
  public shouldRender(nowMs: number): boolean {
    if (!(this.maxFrameRate > 0) || this.lastRenderMs === null) {
      this.lastRenderMs = nowMs;
      return true;
    }

    const intervalMs = 1000 / this.maxFrameRate;
    const elapsedMs = nowMs - this.lastRenderMs;

    // Callbacks arrive with a little jitter, so allow a small tolerance or
    // a 60 cap on a 60 Hz display would skip every other frame.
    if (elapsedMs < intervalMs - TOLERANCE_MS) {
      return false;
    }

    // Keep the schedule on a fixed grid, counting early and late frames
    // against the next one, so a 60 cap on a 144 Hz display averages 60.
    const offsetMs = Math.min(Math.max(elapsedMs - intervalMs, -TOLERANCE_MS), intervalMs);
    this.lastRenderMs = nowMs - offsetMs;
    return true;
  }

  /** Forget the schedule, for example after the loop was stopped. */
  public reset(): void {
    this.lastRenderMs = null;
  }
}
