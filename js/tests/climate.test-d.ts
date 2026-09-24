import { describe, expectTypeOf, it } from "vitest";
import type { BiomeKind, BiomeOptions, VistaEngine } from "../src/types";

describe("climate types", () => {
  it("types the climate temperature and the cold biome", () => {
    expectTypeOf<BiomeOptions["meanTemperatureCelsius"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<"iceArctic">().toMatchTypeOf<BiomeKind>();
    expectTypeOf<VistaEngine["temperatureAt"]>().parameters.toEqualTypeOf<[number, number]>();
    expectTypeOf<VistaEngine["temperatureAt"]>().returns.toEqualTypeOf<number | null>();
  });

  it("rejects other shapes", () => {
    // @ts-expect-error the temperature is a number of degrees, not a name.
    const named: BiomeOptions = { meanTemperatureCelsius: "cold" };
    // @ts-expect-error biome names are camelCase.
    const kebab: BiomeKind = "ice-arctic";
    expectTypeOf(named).toBeObject();
    expectTypeOf(kebab).toBeString();
  });
});
