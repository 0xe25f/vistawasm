import { describe, expectTypeOf, it } from "vitest";
import type {
  RiverOptions,
  VistaEngine,
  WaterMask,
  WaterSound,
  WaterSounds,
  Waterfall
} from "../src/types";

describe("water types", () => {
  it("types the new river options", () => {
    expectTypeOf<RiverOptions["snowmelt"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<RiverOptions["springs"]>().toEqualTypeOf<boolean | undefined>();
    expectTypeOf<RiverOptions["meanders"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<RiverOptions["waterfalls"]>().toEqualTypeOf<boolean | undefined>();
  });

  it("types the water mask, sounds and waterfalls", () => {
    expectTypeOf<WaterMask["data"]>().toEqualTypeOf<Uint8Array>();
    expectTypeOf<VistaEngine["setWaterMask"]>().parameter(0).toEqualTypeOf<WaterMask | null>();
    expectTypeOf<VistaEngine["getWaterSounds"]>().returns.toEqualTypeOf<WaterSounds>();
    expectTypeOf<WaterSounds["waterfall"]>().toEqualTypeOf<WaterSound | null>();
    expectTypeOf<WaterSound["position"]>().toEqualTypeOf<[number, number, number]>();
    expectTypeOf<VistaEngine["getWaterfalls"]>().returns.toEqualTypeOf<Waterfall[]>();
    expectTypeOf<Waterfall["dischargeCubicMetresPerSecond"]>().toEqualTypeOf<number>();
  });

  it("rejects other shapes", () => {
    // @ts-expect-error the mask data is bytes, not an array of numbers.
    const mask: WaterMask = { width: 2, height: 1, data: [0, 1] };
    // @ts-expect-error meanders is a strength from 0 to 1.
    const rivers: RiverOptions = { meanders: "strong" };
    expectTypeOf(mask).toBeObject();
    expectTypeOf(rivers).toBeObject();
  });
});
