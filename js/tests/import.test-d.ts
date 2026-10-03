import { describe, expectTypeOf, it } from "vitest";
import type {
  BiomeKind,
  BiomeMap,
  BundleLoadOptions,
  DensityMask,
  HeightmapImageImportOptions,
  MapKind,
  PaintedMaps,
  RawHeightmapOptions,
  TerrainHandle,
  VistaEngine,
  VistaErrorCode,
  WaterMask
} from "../src/types";
import type {
  biomeMapFromImage,
  decodePng,
  densityMaskFromImage,
  loadBundle,
  loadTerrainFromImages,
  waterMaskFromImage
} from "../src/index";

describe("import types", () => {
  it("types the engine's import methods", () => {
    expectTypeOf<VistaEngine["loadHeightmapImage"]>()
      .parameters.toEqualTypeOf<[Blob | ArrayBuffer | Uint8Array, HeightmapImageImportOptions]>();
    expectTypeOf<VistaEngine["loadHeightmapImage"]>().returns.toEqualTypeOf<Promise<TerrainHandle>>();
    expectTypeOf<VistaEngine["setBiomeMap"]>().parameters.toEqualTypeOf<[BiomeMap | null]>();
    expectTypeOf<VistaEngine["setVegetationMasks"]>()
      .parameters.toEqualTypeOf<[{ trees?: DensityMask | null; grass?: DensityMask | null }]>();
    expectTypeOf<VistaEngine["getPaintedMaps"]>().returns.toEqualTypeOf<PaintedMaps>();
    expectTypeOf<PaintedMaps["water"]>().toEqualTypeOf<WaterMask | undefined>();
  });

  it("types the options", () => {
    expectTypeOf<HeightmapImageImportOptions["metresPerSample"]>().toEqualTypeOf<number>();
    expectTypeOf<HeightmapImageImportOptions["channel"]>().toEqualTypeOf<"luminance" | "r" | "g" | "b" | "a" | undefined>();
    expectTypeOf<BiomeMap["borderSamples"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<BundleLoadOptions["applySettings"]>().toEqualTypeOf<boolean | undefined>();
    expectTypeOf<RawHeightmapOptions["landform"]>().toEqualTypeOf<
      "continental" | "alpine" | "rollingHills" | "archipelago" | "mesaDesert" | "fjords" | "volcanicIsland" | undefined
    >();
    expectTypeOf<"sourceHeight">().toMatchTypeOf<MapKind>();
    expectTypeOf<"INVALID_DEM">().toMatchTypeOf<VistaErrorCode>();
    // @ts-expect-error metresPerSample is required.
    const missing: HeightmapImageImportOptions = { minHeightMetres: 0 };
    expectTypeOf(missing).toBeObject();
  });

  it("types the helpers", () => {
    expectTypeOf<typeof decodePng>().returns.resolves.toHaveProperty("data").toEqualTypeOf<Uint8Array | Uint16Array>();
    expectTypeOf<typeof biomeMapFromImage>().returns.resolves.toEqualTypeOf<{ map: BiomeMap; unmatchedFraction: number }>();
    expectTypeOf<typeof waterMaskFromImage>().returns.resolves.toEqualTypeOf<WaterMask>();
    expectTypeOf<typeof densityMaskFromImage>().returns.resolves.toEqualTypeOf<DensityMask>();
    expectTypeOf<typeof loadBundle>().returns.toEqualTypeOf<Promise<TerrainHandle>>();
    expectTypeOf<typeof loadTerrainFromImages>().returns.toEqualTypeOf<Promise<TerrainHandle>>();
    expectTypeOf<Parameters<typeof biomeMapFromImage>[1]>()
      .exclude<undefined>()
      .toHaveProperty("legend")
      .toEqualTypeOf<{ colour: [number, number, number]; biome: BiomeKind }[] | undefined>();
  });
});
