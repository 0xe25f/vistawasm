import { describe, expectTypeOf, it } from "vitest";
import type { FractalTerrainOptions, LandformKind } from "../src/types";

describe("LandformKind", () => {
  it("accepts every documented landform", () => {
    const names: LandformKind[] = [
      "continental",
      "alpine",
      "rollingHills",
      "archipelago",
      "mesaDesert",
      "fjords",
      "volcanicIsland"
    ];
    expectTypeOf(names).toEqualTypeOf<LandformKind[]>();
    expectTypeOf<FractalTerrainOptions["landform"]>().toEqualTypeOf<LandformKind | undefined>();
  });

  it("rejects other names", () => {
    // @ts-expect-error "glacier" is not a landform.
    const unknown: LandformKind = "glacier";
    // @ts-expect-error landform names are camelCase.
    const kebab: LandformKind = "rolling-hills";
    expectTypeOf(unknown).toBeString();
    expectTypeOf(kebab).toBeString();
  });
});
