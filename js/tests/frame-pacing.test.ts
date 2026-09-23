import { describe, expect, it } from "vitest";
import { FramePacer } from "../src/frame-pacing";

function renderedFrames(pacer: FramePacer, refreshRate: number, seconds: number): number {
  let rendered = 0;

  for (let frame = 0; frame < refreshRate * seconds; frame += 1) {
    if (pacer.shouldRender((frame * 1000) / refreshRate)) {
      rendered += 1;
    }
  }

  return rendered;
}

describe("FramePacer", () => {
  it("renders every frame of a 60 Hz display under the default 60 cap", () => {
    expect(renderedFrames(new FramePacer(), 60, 2)).toBe(120);
  });

  it("holds 60 frames per second on a 144 Hz display", () => {
    const rendered = renderedFrames(new FramePacer(60), 144, 10);

    expect(rendered).toBeGreaterThanOrEqual(598);
    expect(rendered).toBeLessThanOrEqual(602);
  });

  it("renders every frame when uncapped", () => {
    expect(renderedFrames(new FramePacer(0), 144, 1)).toBe(144);
  });

  it("follows a changed cap", () => {
    const pacer = new FramePacer(60);
    pacer.setMaxFrameRate(30);

    expect(renderedFrames(pacer, 120, 2)).toBe(60);
  });
});
