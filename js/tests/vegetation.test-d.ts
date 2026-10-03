import { describe, expectTypeOf, it } from "vitest";
import type { GpuPassTimes, GrassOptions, RenderQualityOptions } from "../src/types";

describe("vegetation density types", () => {
  it("types the forest floor, budgets and distances", () => {
    expectTypeOf<GrassOptions["forestFloor"]>().toEqualTypeOf<boolean | undefined>();
    expectTypeOf<RenderQualityOptions["vegetationDetailMetres"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<RenderQualityOptions["canopyDistanceMetres"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<RenderQualityOptions["maxTreeInstances"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<RenderQualityOptions["maxGrassInstances"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<GpuPassTimes["generation"]>().toEqualTypeOf<number | undefined>();

    const quality: RenderQualityOptions = {
      preset: "balanced",
      vegetationDetailMetres: 300,
      canopyDistanceMetres: 3000,
      maxTreeInstances: 150000,
      maxGrassInstances: 500000
    };
    const grass: GrassOptions = {
      enabled: true,
      style: "billboard-blades",
      density: 4,
      viewDistanceMetres: 220,
      seedOffset: 7331,
      maxInstances: 2000000,
      forestFloor: false
    };
    expectTypeOf(quality).toMatchTypeOf<RenderQualityOptions>();
    expectTypeOf(grass).toMatchTypeOf<GrassOptions>();
  });

  it("rejects other shapes", () => {
    // @ts-expect-error the canopy distance is in metres.
    const named: RenderQualityOptions = { preset: "balanced", canopyDistanceMetres: "far" };
    // @ts-expect-error the forest floor is on or off.
    const floor: Partial<GrassOptions> = { forestFloor: "ferns" };
    expectTypeOf(named).toBeObject();
    expectTypeOf(floor).toBeObject();
  });
});
