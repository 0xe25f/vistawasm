import { describe, expectTypeOf, it } from "vitest";
import type { FractalTerrainOptions, LandformKind, TerrainEdges } from "../src/types";

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

describe("TerrainEdges", () => {
  it("accepts coast and open", () => {
    const edges: TerrainEdges[] = ["coast", "open"];
    expectTypeOf(edges).toEqualTypeOf<TerrainEdges[]>();
    expectTypeOf<FractalTerrainOptions["edges"]>().toEqualTypeOf<TerrainEdges | undefined>();
  });

  it("rejects other values", () => {
    // @ts-expect-error "cliff" is not an edge treatment.
    const unknown: TerrainEdges = "cliff";
    expectTypeOf(unknown).toBeString();
  });
});
