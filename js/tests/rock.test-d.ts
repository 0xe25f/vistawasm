import { describe, expectTypeOf, it } from "vitest";
import type { GpuPassTimes, SurfaceOptions } from "../src/types";

describe("rock outcrop and scree types", () => {
  it("types rockiness, boulders and the boulder distance", () => {
    expectTypeOf<SurfaceOptions["rockiness"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<SurfaceOptions["boulders"]>().toEqualTypeOf<boolean | undefined>();
    expectTypeOf<SurfaceOptions["boulderDistanceMetres"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<GpuPassTimes["boulders"]>().toEqualTypeOf<number | undefined>();

    const surface: SurfaceOptions = {
      rockiness: 1.4,
      boulders: true,
      boulderDistanceMetres: 600,
      materialTints: Array.from({ length: 12 }, () => [1, 1, 1] as [number, number, number])
    };
    expectTypeOf(surface).toMatchTypeOf<SurfaceOptions>();
  });

  it("rejects other shapes", () => {
    // @ts-expect-error rockiness is a number.
    const rocky: SurfaceOptions = { rockiness: "high" };
    // @ts-expect-error boulders are on or off.
    const boulders: SurfaceOptions = { boulders: 1 };
    expectTypeOf(rocky).toBeObject();
    expectTypeOf(boulders).toBeObject();
  });
});
