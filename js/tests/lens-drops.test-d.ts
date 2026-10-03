import { describe, expectTypeOf, it } from "vitest";
import type { WeatherOptions } from "../src/types";

describe("lens drop types", () => {
  it("types the drop count and sizes", () => {
    expectTypeOf<WeatherOptions["lensDropCount"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<WeatherOptions["lensDropMinSize"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<WeatherOptions["lensDropMaxSize"]>().toEqualTypeOf<number | undefined>();

    const weather: WeatherOptions = {
      enabled: true,
      state: "rain",
      lensDrops: true,
      lensDropCount: 120,
      lensDropMinSize: 0.01,
      lensDropMaxSize: 0.08
    };
    expectTypeOf(weather).toMatchTypeOf<WeatherOptions>();
  });

  it("rejects other shapes", () => {
    // @ts-expect-error the count is a number of drops.
    const named: WeatherOptions = { lensDropCount: "many" };
    // @ts-expect-error sizes are fractions of the canvas height, not strings.
    const text: WeatherOptions = { lensDropMaxSize: "5%" };
    expectTypeOf(named).toBeObject();
    expectTypeOf(text).toBeObject();
  });
});
