import { describe, expectTypeOf, it } from "vitest";
import type {
  BuiltInWeatherKind,
  LocalWeather,
  ResolvedWeatherPreset,
  TimeOfDay,
  TimeOfDayOptions,
  VistaEngine,
  WeatherKind,
  WeatherOptions,
  WeatherPreset,
  WeatherState
} from "../src/types";

describe("weather preset types", () => {
  it("accepts built-in and custom preset names", () => {
    const builtIn: WeatherKind = "heavyRain";
    const custom: WeatherKind = "tropicalDownpour";
    expectTypeOf(builtIn).toMatchTypeOf<string>();
    expectTypeOf(custom).toMatchTypeOf<string>();
    expectTypeOf<BuiltInWeatherKind>().toMatchTypeOf<WeatherKind>();
    expectTypeOf<WeatherState["to"]>().toEqualTypeOf<WeatherKind>();

    const weather: WeatherOptions = {
      enabled: true,
      state: "tropicalDownpour",
      regional: true,
      regionSizeKm: 64,
      presets: {
        tropicalDownpour: { extends: "heavyRain", humidity: 1, turbidity: 8, next: { rain: 1 } },
        rain: { humidity: 0.95 }
      }
    };
    expectTypeOf(weather).toMatchTypeOf<WeatherOptions>();
  });

  it("types every preset field as an optional number", () => {
    expectTypeOf<WeatherPreset["cloudCoverage"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<WeatherPreset["cellularity"]>().toEqualTypeOf<number | undefined>();
    expectTypeOf<WeatherPreset["climate"]>().toEqualTypeOf<
      { minCelsius?: number; maxCelsius?: number } | undefined
    >();
    expectTypeOf<ResolvedWeatherPreset["turbidity"]>().toEqualTypeOf<number>();
    expectTypeOf<ResolvedWeatherPreset>().not.toHaveProperty("extends");
    // @ts-expect-error fields are numbers, not strings.
    const wrong: WeatherPreset = { humidity: "damp" };
    expectTypeOf(wrong).toBeObject();
  });

  it("types the new engine members", () => {
    expectTypeOf<VistaEngine["weatherAt"]>().toEqualTypeOf<(x: number, z: number) => LocalWeather | null>();
    expectTypeOf<VistaEngine["advanceWeather"]>().toEqualTypeOf<(seconds: number) => void>();
    expectTypeOf<VistaEngine["setTimeOfDay"]>().toEqualTypeOf<(options: TimeOfDayOptions) => void>();
    expectTypeOf<VistaEngine["getTimeOfDay"]>().toEqualTypeOf<() => TimeOfDay>();
    expectTypeOf<VistaEngine["getWeatherPresets"]>().toEqualTypeOf<
      () => Record<string, ResolvedWeatherPreset>
    >();
    expectTypeOf<LocalWeather["snowDepth"]>().toEqualTypeOf<number>();
    expectTypeOf<TimeOfDay["sunsetHours"]>().toEqualTypeOf<number | null>();
    expectTypeOf<TimeOfDayOptions["dayOfYear"]>().toEqualTypeOf<number | undefined>();
  });
});
