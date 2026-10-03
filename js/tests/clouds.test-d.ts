import { describe, expectTypeOf, it } from "vitest";
import type { CloudsOptions, ResolvedWeatherPreset, WeatherPreset } from "../src/types";

describe("cloud base and mid-level cloud types", () => {
  it("types the new cloud options as optional numbers", () => {
    expectTypeOf<CloudsOptions["baseVariation"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<CloudsOptions["baseLumpiness"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<CloudsOptions["altocumulus"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<CloudsOptions["altostratus"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<CloudsOptions["altoHeightMetres"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<CloudsOptions["altoSpeed"]>().toEqualTypeOf<number | undefined>();

    const mackerel: Partial<CloudsOptions> = { altocumulus: 0.7, altoHeightMetres: 4200, altoSpeed: 1 };
    expectTypeOf(mackerel).toMatchTypeOf<Partial<CloudsOptions>>();
    // @ts-expect-error amounts are numbers, not strings.
    const wrong: Partial<CloudsOptions> = { altostratus: "thick" };
    expectTypeOf(wrong).toBeObject();
  });

  it("types the new preset fields", () => {
    expectTypeOf<WeatherPreset["baseVariation"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<WeatherPreset["baseLumpiness"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<WeatherPreset["altocumulus"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<WeatherPreset["altostratus"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<WeatherPreset["altoHeightMetres"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<WeatherPreset["altoSpeed"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<ResolvedWeatherPreset["altostratus"]>().toEqualTypeOf<number>();
    expectTypeOf<ResolvedWeatherPreset["altoHeightMetres"]>().toEqualTypeOf<number>();

    const veiled: WeatherPreset = { extends: "overcast", altostratus: 0.9, altoHeightMetres: 3500 };
    expectTypeOf(veiled).toMatchTypeOf<WeatherPreset>();
  });
});
