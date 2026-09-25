import { describe, expect, it } from "vitest";
import {
  computeForwardVector,
  computeRightVector,
  computeYawPitchTowards
} from "../src/camera-controls";

describe("computeForwardVector", () => {
  it("looks along positive Z at zero yaw and pitch", () => {
    const [x, y, z] = computeForwardVector(0, 0);

    expect(x).toBeCloseTo(0);
    expect(y).toBeCloseTo(0);
    expect(z).toBeCloseTo(1);
  });

  it("looks straight up at 90 degrees of pitch", () => {
    const [x, y, z] = computeForwardVector(0, Math.PI / 2);

    expect(x).toBeCloseTo(0);
    expect(y).toBeCloseTo(1);
    expect(z).toBeCloseTo(0);
  });

  it("stays a unit vector for arbitrary angles", () => {
    const [x, y, z] = computeForwardVector(1.234, -0.456);
    const length = Math.hypot(x, y, z);

    expect(length).toBeCloseTo(1);
  });
});

describe("computeYawPitchTowards", () => {
  it("turns the forward vector towards the target", () => {
    const from: [number, number, number] = [10, 50, -20];
    const to: [number, number, number] = [-30, 20, 60];
    const [yaw, pitch] = computeYawPitchTowards(from, to);
    const forward = computeForwardVector(yaw, pitch);
    const length = Math.hypot(to[0] - from[0], to[1] - from[1], to[2] - from[2]);

    expect(forward[0]).toBeCloseTo((to[0] - from[0]) / length);
    expect(forward[1]).toBeCloseTo((to[1] - from[1]) / length);
    expect(forward[2]).toBeCloseTo((to[2] - from[2]) / length);
  });
});

describe("computeRightVector", () => {
  it("is perpendicular to the forward vector", () => {
    const yaw = 0.73;
    const forward = computeForwardVector(yaw, 0);
    const right = computeRightVector(yaw);
    const dot = forward[0] * right[0] + forward[1] * right[1] + forward[2] * right[2];

    expect(dot).toBeCloseTo(0);
  });

  it("stays a unit vector", () => {
    const [x, y, z] = computeRightVector(2.1);

    expect(Math.hypot(x, y, z)).toBeCloseTo(1);
  });
});
