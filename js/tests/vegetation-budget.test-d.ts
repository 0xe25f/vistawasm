import { describe, expectTypeOf, it } from "vitest";
import type { GpuPassTimes, RenderQualityOptions, RenderStats } from "../src/types";

describe("vegetation course correction types", () => {
  it("types the triangle budget, the grass detail radius and split timing", () => {
    expectTypeOf<RenderQualityOptions["maxTreeTriangles"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<RenderQualityOptions["grassDetailMetres"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<RenderQualityOptions["splitTreeTiming"]>().toEqualTypeOf<boolean | undefined>();
    expectTypeOf<RenderStats["treeTriangles"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<GpuPassTimes["treeMeshes"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<GpuPassTimes["understorey"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<GpuPassTimes["treeImpostors"]>().toEqualTypeOf<number | undefined>();

    const quality: RenderQualityOptions = {
      preset: "balanced",
      maxTreeTriangles: 2500000,
      grassDetailMetres: 45,
      splitTreeTiming: true
    };
    expectTypeOf(quality).toMatchTypeOf<RenderQualityOptions>();
  });

  it("rejects other shapes", () => {
    // @ts-expect-error the triangle budget is a number.
    const triangles: RenderQualityOptions = { preset: "balanced", maxTreeTriangles: "many" };
    // @ts-expect-error split timing is on or off.
    const split: RenderQualityOptions = { preset: "balanced", splitTreeTiming: 1 };
    expectTypeOf(triangles).toBeObject();
    expectTypeOf(split).toBeObject();
  });
});
