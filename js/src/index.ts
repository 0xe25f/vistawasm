import { VistaWasmError, checkOptions, cloneOptions, invalid, toVistaWasmError, type OptionKeys } from "./errors.js";
import { TERRAIN_SIZE_REASON } from "./codec.js";
import { assertVistaWasmSupport, detectVistaWasmSupport } from "./feature-detect.js";
import { DEFAULT_MAX_FRAME_RATE, FramePacer } from "./frame-pacing.js";
import { importGlue } from "./import-glue.js";
import { OPTION_KEYS } from "./option-keys.js";
import { MAP_KINDS, TREE_SPECIES, checkSize } from "./map-export.js";
import {
  biomeMapFromImage,
  channelOf,
  decodeImage,
  densityMaskFromImage,
  MAX_TERRAIN_SIDE,
  readBundle,
  waterMaskFromImage
} from "./map-import.js";
import type {
  AtmosphereOptions,
  BiomeKind,
  BiomeMap,
  BundleLoadOptions,
  DensityMask,
  HeightmapImageImportOptions,
  TerrainImages,
  ImageSource,
  PaintedMaps,
  VegetationMasks,
  BiomeOptions,
  CameraOptions,
  CloudsOptions,
  DebugView,
  DemFetchOptions,
  DemLoadOptions,
  ExportHeightmapOptions,
  ExportMapOptions,
  ExportTreesOptions,
  ExportedMap,
  FloraOptions,
  MapKind,
  FractalTerrainOptions,
  GrassOptions,
  MistOptions,
  RawExportedMap,
  RawHeightmapOptions,
  RenderQualityOptions,
  RenderStats,
  SnapshotOptions,
  ShadowOptions,
  SunOptions,
  SurfaceOptions,
  TerrainHandle,
  TextureTarget,
  TreeModel,
  TreePlacement,
  TreeRecord,
  TreeSpecies,
  VistaEngine,
  VistaEngineOptions,
  VistaEventListener,
  VistaEventMap,
  VistaEventName,
  VistaOptionsSnapshot,
  VistaWasmGeneratedModule,
  VistaWasmInitOptions,
  VistaWasmRawEngine,
  WaterMask,
  WaterOptions,
  WaterSound,
  WaterSounds,
  Waterfall,
  WaterInflow,
  WeatherKind,
  WeatherOptions,
  WeatherState,
  ResolvedWeatherPreset,
  LocalWeather,
  TimeOfDayOptions,
  TimeOfDay
} from "./types.js";

/** Edge length, in texels, of every replaceable texture layer. */
export const TEXTURE_LAYER_SIZE = 512;

/** Most custom weather presets one `setWeather()` call may add. */
export const MAX_WEATHER_PRESETS = 64;

/** Longest weather preset name, in UTF-16 code units. */
export const MAX_PRESET_NAME_LENGTH = 64;

/** Most vertices a custom tree model may have, as the engine allows. */
const MAX_TREE_MODEL_VERTICES = 65_536;

/** Largest canvas side `resize()` accepts, in CSS pixels. */
const MAX_CANVAS_SIDE = 8_192;

/**
 * Largest buffer `loadRawHeightmap()` accepts: 2048 x 2048 float32
 * samples (16 MiB), the largest map (`MAX_TERRAIN_SIDE`). A buffer longer than its
 * options need, but within this, is read in part with a warning.
 */
export const MAX_RAW_HEIGHTMAP_BYTES = MAX_TERRAIN_SIDE * MAX_TERRAIN_SIDE * 4;

/**
 * Largest GeoTIFF `loadDemFromArrayBuffer()` accepts and `fetchDemBytes()`
 * reads: a 2048 x 2048 float32 map and 64 MiB for its tags (80 MiB).
 */
export const MAX_DEM_BYTES = MAX_TERRAIN_SIDE * MAX_TERRAIN_SIDE * 4 + 64 * 2 ** 20;

/** Most trees `setTreeInstances()` takes, as the engine allows. */
const MAX_CUSTOM_TREES = 1_000_000;

/** Bytes per sample of each raw sample format. */
const SAMPLE_BYTES: Record<string, number> = { uint16: 2, int16: 2, float32: 4 };

const DEBUG_VIEWS: readonly DebugView[] = ["none", "height", "slope", "normals", "lod", "flow", "materials", "no-data", "biomes"];

const EVENT_NAMES: readonly VistaEventName[] = ["ready", "progress", "warning", "terrainLoaded", "stats", "fatalError", "deviceLost", "gpuError", "weatherChanged"];


export { VistaWasmError, detectVistaWasmSupport };
export { MAX_OPTIONS_DEPTH } from "./errors.js";
export { MAX_IMAGE_SIDE } from "./codec.js";
export type * from "./types.js";
export * from "./camera-controls.js";
export * from "./terrain-export.js";
export {
  MAP_KINDS,
  downloadBundle,
  encodePng,
  encodeRaw,
  exportBundle,
  treesToCsv,
  treesToJson
} from "./map-export.js";
export type { BundleOptions, PngBlob, PngOptions, RawOptions } from "./map-export.js";
export {
  BIOME_COLOURS,
  BIOME_KINDS,
  BUNDLE_LIMITS,
  MAX_LEGEND_COLOURS,
  MAX_TERRAIN_SIDE,
  biomeMapFromImage,
  densityMaskFromImage,
  imageSize,
  waterMaskFromImage
} from "./map-import.js";
export { decodePng } from "./png-decode.js";
export type { DecodedPng } from "./png-decode.js";

let loadedModule: VistaWasmGeneratedModule | null = null;
let loadingModule: Promise<VistaWasmGeneratedModule> | null = null;

/**
 * Load the generated WASM module once.
 */
export async function initialiseVistaWasm(
  options: VistaWasmInitOptions = {}
): Promise<void> {
  const { moduleUrl } = checkOptions("initialiseVistaWasm()", options);

  if (moduleUrl !== undefined && typeof moduleUrl !== "string" && !(moduleUrl instanceof URL)) {
    throw new TypeError("initialiseVistaWasm() moduleUrl must be a string or a URL.");
  }

  if (loadedModule) {
    return;
  }

  loadingModule ??= loadWasmModule(options).catch((error: unknown) => {
    // A failed load is not cached, so a later call can try again.
    loadingModule = null;
    throw error;
  });
  loadedModule = await loadingModule;
}

/**
 * Create a VistaWASM engine for an existing canvas.
 */
export async function createVistaEngine(
  canvas: HTMLCanvasElement,
  options: VistaEngineOptions = {}
): Promise<VistaEngine> {
  assertCanvas(canvas);
  assertVistaWasmSupport();
  const warnings: string[] = [];
  const normalised = normaliseEngineOptions(options, warnings);
  await initialiseVistaWasm();

  if (!loadedModule) {
    throw new VistaWasmError("INTERNAL_ERROR", "VistaWASM failed to load its WASM module.");
  }

  try {
    const raw = await loadedModule.VistaEngine.create(canvas, normalised);
    const engine = new VistaEngineWrapper(canvas, raw, normalised);
    engine.setFrameRateCap(normalised.quality);
    // Warnings are emitted on a later task, so listeners added once the
    // engine is returned hear them.
    setTimeout(() => engine.warn(warnings.join("\n")));
    engine.emit("ready", undefined);
    return engine;
  } catch (error) {
    throw toVistaWasmError(error);
  }
}

/**
 * Fetch DEM bytes from a URL.
 */
export async function fetchDemBytes(
  url: string,
  options: DemFetchOptions = {}
): Promise<ArrayBuffer> {
  if (typeof url !== "string" && !((url as unknown) instanceof URL)) {
    throw new TypeError("fetchDemBytes() expects the DEM's URL as a string or a URL.");
  }

  const { headers, signal, maxBytes = MAX_DEM_BYTES } = checkOptions<DemFetchOptions>("fetchDemBytes()", options);

  if (!Number.isInteger(maxBytes) || maxBytes < 1 || maxBytes > MAX_DEM_BYTES) {
    throw invalid(`fetchDemBytes() maxBytes must be a whole number from 1 to ${MAX_DEM_BYTES}, but it is ${String(maxBytes)}.`);
  }

  const response = await fetch(url, { headers, signal });

  if (!response.ok) {
    await response.body?.cancel();
    throw new VistaWasmError(
      "DEM_FETCH_FAILED",
      `Could not fetch DEM data. HTTP ${response.status}.`
    );
  }

  const tooLong = (bytes: number | string) => new VistaWasmError(
    "DEM_FETCH_FAILED",
    `Could not fetch DEM data: the response is ${bytes} bytes, over the limit of ${maxBytes}. Pass a larger maxBytes (at most ${MAX_DEM_BYTES}) if the file is meant to be this large.`
  );
  const declared = Number(response.headers.get("content-length") ?? NaN);

  // Refused before reading: the header says the body is too long.
  if (declared > maxBytes) {
    await response.body?.cancel();
    throw tooLong(declared);
  }

  if (!response.body) {
    return new ArrayBuffer(0);
  }

  // Read in pieces, so a body longer than its header (or without one)
  // is cancelled as soon as it passes the limit.
  const reader = response.body.getReader();
  const pieces: Uint8Array[] = [];
  let length = 0;

  for (let next = await reader.read(); !next.done; next = await reader.read()) {
    length += next.value.byteLength;

    if (length > maxBytes) {
      await reader.cancel();
      throw tooLong(`over ${maxBytes}`);
    }

    pieces.push(next.value);
  }

  const bytes = new Uint8Array(length);
  let at = 0;

  for (const piece of pieces) {
    bytes.set(piece, at);
    at += piece.byteLength;
  }

  return bytes.buffer;
}

class VistaEngineWrapper implements VistaEngine {
  private readonly listeners = new Map<VistaEventName, Set<(payload: unknown) => void>>();

  private animationFrameId: number | null = null;

  private running = false;

  private disposed = false;

  /**
   * The WebAssembly trap that stopped the engine, if one did. Release
   * builds abort on a panic, which leaves the WASM instance unusable, so
   * every later call is refused.
   */
  private trap: Error | null = null;

  /** Whether `"deviceLost"` has been emitted. */
  private lost = false;

  /**
   * Tracks any async call currently in flight against the underlying WASM
   * object (for example `generateFractal()`, which can now span several
   * animation frames while GPU erosion runs). WASM-bindgen forbids any
   * other call reaching the same object while an async method has not yet
   * resolved, so every sync per-frame call (`setCamera()`, `renderOnce()`,
   * and so on) must check this and skip rather than reenter the object.
   */
  private pendingCall: Promise<unknown> | null = null;

  private lastWeather: WeatherKind | null = null;

  /** The last frame drawn, which the stats may not have been read for. */
  private lastFrameIndex = 0;

  /** The painted maps in effect, copied as they were set. */
  private painted: PaintedMaps = {};

  private readonly pacer = new FramePacer();

  private lastStats: RenderStats = {
    frameIndex: 0,
    frameTimeMs: 0,
    terrainTriangles: 0,
    floraInstances: 0,
    grassInstances: 0,
    clipmapLevels: 0
  };

  /**
   * @param snapshot The options the engine was created with; each setter
   * the engine accepts replaces its part, for `getOptionsSnapshot()`.
   */
  public constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly raw: VistaWasmRawEngine,
    private readonly snapshot: VistaOptionsSnapshot = {}
  ) {}

  public async generateFractal(options: FractalTerrainOptions): Promise<TerrainHandle> {
    this.ensureLive();
    const warnings: string[] = [];
    options = normaliseFractalOptions(cloneOptions("generateFractal()", options, OPTION_KEYS.terrain, warnings));
    this.emit("progress", {
      phase: "fractal",
      progress: 0
    });
    const handle = await this.callAsync<TerrainHandle>(() =>
      this.raw.generateFractal(options, (phase, progress) => {
        this.emit("progress", { phase, progress });
      })
    );
    this.snapshot.terrain = options;
    delete this.snapshot.landform;
    this.painted = {};
    handle.metadata.warnings.unshift(...warnings);
    this.emitWarnings(handle);
    this.emit("terrainLoaded", handle);
    this.emit("progress", {
      phase: "fractal",
      progress: 1
    });
    return handle;
  }

  public async loadDemFromUrl(
    url: string,
    options: DemFetchOptions = {}
  ): Promise<TerrainHandle> {
    this.ensureLive();
    const { headers, signal, maxBytes, ...load } = checkOptions<DemFetchOptions>("loadDemFromUrl()", options);
    const buffer = await fetchDemBytes(url, { headers, signal, maxBytes });
    return this.loadDemFromArrayBuffer(buffer, load);
  }

  public async loadDemFromArrayBuffer(
    buffer: ArrayBuffer,
    options: DemLoadOptions = {}
  ): Promise<TerrainHandle> {
    this.ensureLive();
    checkBuffer("loadDemFromArrayBuffer()", buffer);

    if (buffer.byteLength > MAX_DEM_BYTES) {
      throw new VistaWasmError(
        "DEM_FORMAT_UNSUPPORTED",
        `loadDemFromArrayBuffer() was given ${buffer.byteLength} bytes, more than the ${MAX_DEM_BYTES} a GeoTIFF of at most ${MAX_TERRAIN_SIDE} x ${MAX_TERRAIN_SIDE} samples needs.`
      );
    }

    const warnings: string[] = [];
    options = cloneOptions("loadDemFromArrayBuffer()", options, OPTION_KEYS.dem, warnings);
    const handle = await this.callAsync<TerrainHandle>(() =>
      this.raw.loadDemFromArrayBuffer(buffer, options)
    );
    delete this.snapshot.terrain;
    delete this.snapshot.landform;
    this.painted = {};
    handle.metadata.warnings.unshift(...warnings);
    this.emitWarnings(handle);
    this.emit("terrainLoaded", handle);
    return handle;
  }

  public async loadRawHeightmap(
    buffer: ArrayBuffer,
    options: RawHeightmapOptions
  ): Promise<TerrainHandle> {
    this.ensureLive();
    checkBuffer("loadRawHeightmap()", buffer);
    const warnings: string[] = [];
    options = cloneOptions("loadRawHeightmap()", options, OPTION_KEYS.raw, warnings);
    checkRawLength(buffer, options);
    const handle = await this.callAsync<TerrainHandle>(() =>
      this.raw.loadRawHeightmap(buffer, options)
    );
    delete this.snapshot.terrain;
    this.snapshot.landform = options.landform;
    this.painted = {};
    handle.metadata.warnings.unshift(...warnings);
    this.emitWarnings(handle);
    this.emit("terrainLoaded", handle);
    return handle;
  }

  public async loadHeightmapImage(
    source: ImageSource,
    options: HeightmapImageImportOptions
  ): Promise<TerrainHandle> {
    this.ensureLive();
    const warnings: string[] = [];
    const { buffer, raw } = await heightmapFromImage(source, options, (message: string) => warnings.push(message));
    const handle = await this.loadRawHeightmap(buffer, raw);
    warnings.forEach((message) => this.emit("warning", { message }));
    return handle;
  }

  public setCamera(camera: CameraOptions): void {
    camera = this.options("setCamera()", camera, OPTION_KEYS.engine.camera);

    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setCamera(camera));
    this.snapshot.camera = camera;
  }

  public setSun(sun: SunOptions): void {
    sun = this.options("setSun()", sun, OPTION_KEYS.engine.sun);

    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setSun(sun));
    this.snapshot.sun = sun;
  }

  public setAtmosphere(atmosphere: AtmosphereOptions): void {
    atmosphere = this.options("setAtmosphere()", atmosphere, OPTION_KEYS.engine.atmosphere);

    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setAtmosphere(atmosphere));
    this.snapshot.atmosphere = atmosphere;
  }

  public setWater(water: WaterOptions): void {
    water = this.options("setWater()", water, OPTION_KEYS.engine.water);

    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setWater(water));
    this.snapshot.water = water;
  }

  public setFlora(flora: FloraOptions): void {
    const normalised = normaliseFloraOptions(this.options<FloraOptions>("setFlora()", flora, OPTION_KEYS.engine.flora));

    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setFlora(normalised));
    this.snapshot.flora = normalised;
  }

  public setGrass(grass: GrassOptions): void {
    const normalised = normaliseGrassOptions(this.options<GrassOptions>("setGrass()", grass, OPTION_KEYS.engine.grass));

    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setGrass(normalised));
    this.snapshot.grass = normalised;
  }

  public setClouds(clouds: CloudsOptions): void {
    const normalised = normaliseCloudsOptions(this.options<CloudsOptions>("setClouds()", clouds, OPTION_KEYS.engine.clouds));

    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setClouds(normalised));
    this.snapshot.clouds = normalised;
  }

  public setMist(mist: MistOptions): void {
    const normalised = normaliseMistOptions(this.options<MistOptions>("setMist()", mist, OPTION_KEYS.engine.mist));

    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setMist(normalised));
    this.snapshot.mist = normalised;
  }

  public setRenderQuality(quality: RenderQualityOptions): void {
    quality = this.options("setRenderQuality()", quality, OPTION_KEYS.engine.quality);

    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setRenderQuality(quality));
    this.snapshot.quality = quality;
    this.setFrameRateCap(quality);
  }

  /** Called after the engine accepted the options, so the cap is valid. */
  public setFrameRateCap(quality: RenderQualityOptions | undefined): void {
    this.pacer.setMaxFrameRate(quality?.maxFrameRate ?? DEFAULT_MAX_FRAME_RATE);
  }

  public setBiomes(biomes: BiomeOptions): void {
    const normalised = normaliseBiomeOptions(this.options<BiomeOptions>("setBiomes()", biomes, OPTION_KEYS.engine.biomes));

    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setBiomes(normalised));
    this.snapshot.biomes = normalised;
  }

  public biomeAt(x: number, z: number): BiomeKind | undefined {
    if (this.pendingCall || !Number.isFinite(x) || !Number.isFinite(z)) {
      return undefined;
    }

    return this.call(() => this.raw.biomeAt(x, z)) as BiomeKind | undefined;
  }

  public temperatureAt(x: number, z: number): number | null {
    if (!Number.isFinite(x) || !Number.isFinite(z)) {
      throw new TypeError("temperatureAt() expects finite x and z world positions in metres.");
    }

    if (this.pendingCall) {
      return null;
    }

    return this.call(() => this.raw.temperatureAt(x, z)) ?? null;
  }

  public setWaterMask(mask: WaterMask | null): void {
    if (mask === null) {
      this.callWhenIdle(() => this.raw.setWaterMask(0, 0, undefined));
      return;
    }

    checkMask("setWaterMask()", mask);
    let warning: string | undefined;
    this.callWhenIdle(() => {
      warning = this.raw.setWaterMask(mask.width, mask.height, mask.data);
    });
    this.painted.water = copyMask(mask);

    if (warning) {
      this.emit("warning", { message: warning });
    }
  }

  public setBiomeMap(map: BiomeMap | null): void {
    if (map === null) {
      this.callWhenIdle(() => this.raw.setBiomeMap(0, 0, undefined, 0));
      delete this.painted.biome;
      return;
    }

    checkMask("setBiomeMap()", map);
    const border = map.borderSamples ?? 3;

    if (!Number.isInteger(border) || border < 0 || border > 8) {
      throw invalid(`setBiomeMap() borderSamples must be a whole number from 0 to 8, but it is ${String(border)}.`);
    }

    let warnings = "";
    this.callWhenIdle(() => {
      warnings = this.raw.setBiomeMap(map.width, map.height, map.data, border);
    });
    this.painted.biome = { ...copyMask(map), borderSamples: border };
    warnings.split("\n").filter(Boolean).forEach((message) => this.emit("warning", { message }));
  }

  public setVegetationMasks(masks: VegetationMasks): void {
    if (!masks || typeof masks !== "object") {
      throw new TypeError("setVegetationMasks() expects { trees?, grass? }, each a DensityMask or null.");
    }

    const kinds = (["trees", "grass"] as const).filter((kind) => masks[kind] !== undefined);
    // Both are checked before either is set.
    kinds.forEach((kind) => masks[kind] && checkMask(`setVegetationMasks() ${kind}`, masks[kind]));

    for (const kind of kinds) {
      const mask = masks[kind];
      let warning: string | undefined;
      this.callWhenIdle(() => {
        warning = this.raw.setVegetationMask(kind === "grass", mask?.width ?? 0, mask?.height ?? 0, mask?.data);
      });

      if (mask) {
        this.painted[kind] = copyMask(mask);
      } else {
        delete this.painted[kind];
      }

      if (warning) {
        this.emit("warning", { message: warning });
      }
    }
  }

  public getPaintedMaps(): PaintedMaps {
    this.ensureLive();
    const copy: PaintedMaps = {};

    for (const [kind, map] of Object.entries(this.painted) as [keyof PaintedMaps, BiomeMap][]) {
      copy[kind] = { ...map, data: map.data.slice() };
    }

    return copy;
  }

  public getWaterSounds(x: number, y: number, z: number): WaterSounds {
    if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(z)) {
      throw new TypeError("getWaterSounds() expects finite x, y and z world positions in metres.");
    }

    const none: WaterSounds = { river: null, waterfall: null, lakeShore: null, surf: null };

    if (this.pendingCall) {
      return none;
    }

    const packed = this.call<Float32Array>(() => this.raw.getWaterSounds(x, y, z));
    const sound = (kind: number): WaterSound | null => {
      const at = kind * 5;
      return Number.isNaN(packed[at])
        ? null
        : {
          distanceMetres: packed[at],
          loudness: packed[at + 1],
          position: [packed[at + 2], packed[at + 3], packed[at + 4]]
        };
    };

    return { river: sound(0), waterfall: sound(1), lakeShore: sound(2), surf: sound(3) };
  }

  public getWaterfalls(): Waterfall[] {
    if (this.pendingCall) {
      return [];
    }

    const packed = this.call<Float32Array>(() => this.raw.getWaterfalls());
    const falls: Waterfall[] = [];

    for (let at = 0; at + 6 <= packed.length; at += 6) {
      falls.push({
        position: [packed[at], packed[at + 1], packed[at + 2]],
        heightMetres: packed[at + 3],
        widthMetres: packed[at + 4],
        dischargeCubicMetresPerSecond: packed[at + 5]
      });
    }

    return falls;
  }

  public getInflows(): WaterInflow[] {
    if (this.pendingCall) {
      return [];
    }

    const packed = this.call<Float32Array>(() => this.raw.getInflows());
    const inflows: WaterInflow[] = [];

    for (let at = 0; at + 4 <= packed.length; at += 4) {
      inflows.push({
        position: [packed[at], packed[at + 1], packed[at + 2]],
        dischargeCubicMetresPerSecond: packed[at + 3]
      });
    }

    return inflows;
  }

  public setDebugView(debugView: DebugView): void {
    if (!DEBUG_VIEWS.includes(debugView)) {
      throw new TypeError(`setDebugView() expects one of ${DEBUG_VIEWS.join(", ")}, but it was given ${String(debugView).slice(0, 80)}.`);
    }

    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setDebugView(debugView));
  }

  public setWeather(weather: WeatherOptions): void {
    const warnings: string[] = [];
    const normalised = normaliseWeather("setWeather()", weather, warnings);
    this.warn(warnings.join("\n"));

    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setWeather(normalised));
    this.snapshot.weather = normalised;
  }

  public getWeather(): WeatherState | undefined {
    if (this.pendingCall) {
      return undefined;
    }

    return this.call<WeatherState | undefined>(() => this.raw.getWeather());
  }

  public getWeatherPresets(): Record<string, ResolvedWeatherPreset> {
    if (this.pendingCall) {
      return {};
    }

    return this.call(() => this.raw.getWeatherPresets()) as Record<string, ResolvedWeatherPreset>;
  }

  public weatherAt(x: number, z: number): LocalWeather | null {
    if (!Number.isFinite(x) || !Number.isFinite(z)) {
      throw new TypeError("weatherAt() expects finite x and z world positions in metres.");
    }

    if (this.pendingCall) {
      return null;
    }

    const packed = this.call<Float32Array>(() => this.raw.weatherAt(x, z));

    if (packed.length < 7) {
      return null;
    }

    return {
      coverage: packed[0],
      precipitation: packed[1],
      storminess: packed[2],
      humidity: packed[3],
      wetness: packed[4],
      puddles: packed[5],
      snowDepth: packed[6]
    };
  }

  public advanceWeather(seconds: number): void {
    if (typeof seconds !== "number" || !Number.isFinite(seconds) || seconds < 0 || seconds > 86400) {
      throw new VistaWasmError(
        "OPTIONS_INVALID",
        `advanceWeather() expects 0 to 86400 seconds, but it was given ${String(seconds)}.`
      );
    }

    this.callWhenIdle(() => this.raw.advanceWeather(seconds));
  }

  public setTimeOfDay(options: TimeOfDayOptions): void {
    if (!options || typeof options !== "object") {
      throw new TypeError("setTimeOfDay() expects an options object.");
    }

    const {
      enabled = false,
      hours = 12,
      dayLengthMinutes = 24,
      latitudeDegrees = 45,
      dayOfYear = 172
    } = options;

    for (const [name, value] of Object.entries({ hours, dayLengthMinutes, latitudeDegrees })) {
      if (typeof value !== "number") {
        throw new TypeError(`setTimeOfDay() expects ${name} to be a number.`);
      }
    }

    if (typeof enabled !== "boolean") {
      throw new TypeError("setTimeOfDay() expects enabled to be true or false.");
    }

    if (!Number.isInteger(dayOfYear) || dayOfYear < 1 || dayOfYear > 366) {
      throw new VistaWasmError(
        "OPTIONS_INVALID",
        `timeOfDay.dayOfYear must be a whole number from 1 to 366, but it is ${String(dayOfYear)}.`
      );
    }

    this.callWhenIdle(() =>
      this.raw.setTimeOfDay(enabled, hours, dayLengthMinutes, latitudeDegrees, dayOfYear)
    );
  }

  public getTimeOfDay(): TimeOfDay {
    const packed = this.call<Float32Array>(() => this.raw.getTimeOfDay());
    const hoursOrNull = (value: number) => (Number.isNaN(value) ? null : value);
    return {
      hours: packed[0],
      sunAzimuthDegrees: packed[1],
      sunElevationDegrees: packed[2],
      sunriseHours: hoursOrNull(packed[3]),
      sunsetHours: hoursOrNull(packed[4])
    };
  }

  public setShadows(shadows: ShadowOptions): void {
    shadows = this.options("setShadows()", shadows, OPTION_KEYS.engine.shadows);

    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setShadows(shadows));
    this.snapshot.shadows = shadows;
  }

  public setSurface(surface: SurfaceOptions): void {
    surface = this.options("setSurface()", surface, OPTION_KEYS.engine.surface);

    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setSurface(surface));
    this.snapshot.surface = surface;
  }

  public setTreeModel(species: TreeSpecies, model: TreeModel): void {
    assertSpecies(species);

    if (!model || typeof model !== "object") {
      throw new TypeError("setTreeModel() expects a model with positions, normals, uvs, and indices.");
    }

    // Checked before the arrays are copied into the engine.
    if (model.positions?.length > MAX_TREE_MODEL_VERTICES * 3) {
      throw invalid(
        `setTreeModel() positions may hold at most ${MAX_TREE_MODEL_VERTICES} vertices (${MAX_TREE_MODEL_VERTICES * 3} numbers), but it holds ${model.positions.length} numbers.`
      );
    }

    const positions = toFloat32("positions", model.positions);
    const normals = toFloat32("normals", model.normals);
    const uvs = toFloat32("uvs", model.uvs);
    const indices = toUint32("indices", model.indices);
    const layers = model.textureLayers === undefined ? undefined : toFloat32("textureLayers", model.textureLayers);
    const wind = model.wind === undefined ? undefined : toFloat32("wind", model.wind);
    this.callWhenIdle(() =>
      this.raw.setTreeModel(species, positions, normals, uvs, indices, layers, wind)
    );
  }

  public resetTreeModel(species: TreeSpecies): void {
    assertSpecies(species);
    this.callWhenIdle(() => this.raw.resetTreeModel(species));
  }

  public setTreeInstances(trees: TreePlacement[] | undefined): void {
    if (trees === undefined) {
      this.callWhenIdle(() => this.raw.setTreeInstances(undefined));
      return;
    }

    if (!Array.isArray(trees)) {
      throw new TypeError("setTreeInstances() expects an array of trees or undefined.");
    }

    // Checked before anything is allocated for them.
    if (trees.length > MAX_CUSTOM_TREES) {
      throw invalid(`setTreeInstances() takes at most ${MAX_CUSTOM_TREES} trees, but it was given ${trees.length}.`);
    }

    // Nine floats per tree keeps this one boundary crossing, however many
    // trees there are.
    const packed = new Float32Array(trees.length * 9);

    trees.forEach((tree, index) => {
      const species = TREE_SPECIES.indexOf(tree?.species);

      if (species < 0) {
        throw new TypeError(
          `Tree ${index} has an unknown species. Use one of: ${TREE_SPECIES.join(", ")}.`
        );
      }

      if (tree.ground !== undefined && typeof tree.ground !== "boolean") {
        throw new TypeError(`Tree ${index} ground must be true, false or undefined.`);
      }

      // A Float32Array would turn strings into numbers or NaN without a word.
      if (![tree.x, tree.y, tree.z, tree.scale ?? 1, tree.rotation ?? 0, tree.tint ?? 0.5, tree.dryness ?? 0].every((value) => typeof value === "number")) {
        throw new TypeError(`Tree ${index} needs numbers for x, y and z, and for scale, rotation, tint and dryness when they are given.`);
      }

      packed.set(
        [
          tree.x,
          tree.y,
          tree.z,
          tree.scale ?? 1,
          tree.rotation ?? 0,
          tree.tint ?? 0.5,
          species,
          tree.dryness ?? 0,
          tree.ground ? 1 : 0
        ],
        index * 9
      );
    });
    this.callWhenIdle(() => this.raw.setTreeInstances(packed));
  }

  public replaceTexture(
    target: TextureTarget,
    layer: number,
    rgba: Uint8Array | Uint8ClampedArray
  ): void {
    if (target !== "terrainAlbedo" && target !== "terrainNormal" && target !== "flora") {
      throw new TypeError('replaceTexture() target must be "terrainAlbedo", "terrainNormal", or "flora".');
    }

    // The engine takes the layer as an unsigned 32-bit number, which
    // would wrap larger values round to a valid layer.
    if (!Number.isInteger(layer) || layer < 0 || layer > 255) {
      throw new TypeError("replaceTexture() layer must be a whole number from 0 to 255; see docs/hooks.md for each target's layers.");
    }

    if (!(rgba instanceof Uint8Array || rgba instanceof Uint8ClampedArray)) {
      throw new TypeError("replaceTexture() expects RGBA texels in a Uint8Array.");
    }

    if (rgba.length !== TEXTURE_LAYER_SIZE * TEXTURE_LAYER_SIZE * 4) {
      throw invalid(
        `replaceTexture() expects ${TEXTURE_LAYER_SIZE} x ${TEXTURE_LAYER_SIZE} RGBA texels (${TEXTURE_LAYER_SIZE * TEXTURE_LAYER_SIZE * 4} bytes), but it was given ${rgba.length} bytes.`
      );
    }

    const bytes = new Uint8Array(rgba.buffer, rgba.byteOffset, rgba.byteLength);
    this.callWhenIdle(() => this.raw.replaceTexture(target, layer, bytes));
  }

  public resetTextures(): void {
    this.callWhenIdle(() => this.raw.resetTextures());
  }

  public renderOnce(): RenderStats {
    if (this.pendingCall) {
      return this.lastStats;
    }

    const start = performance.now();
    const stats = this.call<RenderStats>(() => this.raw.renderOnce());
    this.reportGpuEvents();

    // An unchanged frame index means the GPU was still busy with earlier
    // frames, so the engine skipped this one rather than queue it.
    if (stats.frameIndex === this.lastFrameIndex) {
      return this.lastStats;
    }

    this.lastFrameIndex = stats.frameIndex;
    this.report(stats, start);
    return stats;
  }

  /**
   * One frame of the render loop. Its stats object is only built when
   * something listens for `"stats"` or the weather changed, so a loop
   * nothing watches makes no garbage each frame.
   */
  private renderLoopFrame(): void {
    if (!this.raw.renderFrame) {
      this.renderOnce();
      return;
    }

    if (this.pendingCall) {
      return;
    }

    const start = performance.now();
    const frame = this.call<number>(() => this.raw.renderFrame!());
    this.reportGpuEvents();
    const frameIndex = Math.abs(frame);

    // As in `renderOnce()`: the engine skipped a frame it could not queue.
    if (frameIndex === this.lastFrameIndex) {
      return;
    }

    this.lastFrameIndex = frameIndex;

    if (frame < 0 || this.listeners.get("stats")?.size) {
      this.report(this.call<RenderStats>(() => this.raw.getStats()), start);
    }
  }

  /** Finish a drawn frame's stats, keep them, and emit their events. */
  private report(stats: RenderStats, start: number): void {
    stats.frameTimeMs = performance.now() - start;
    stats.weather ??= null;
    const weatherChanged = stats.weather !== this.lastWeather;
    this.lastWeather = stats.weather;
    this.lastStats = stats;
    this.emit("stats", stats);

    if (weatherChanged) {
      this.emit("weatherChanged", stats.weather);
    }
  }

  public start(): void {
    this.ensureLive();

    if (this.running) {
      return;
    }

    this.running = true;
    this.pacer.reset();
    const tick = (now: number) => {
      if (!this.running) {
        return;
      }

      if (!this.pacer.shouldRender(now)) {
        this.animationFrameId = window.requestAnimationFrame(tick);
        return;
      }

      try {
        this.renderLoopFrame();
      } catch (error) {
        const vistaError = toVistaWasmError(error);

        // A trap and a lost device have already been reported, once.
        if (!this.trap && vistaError.code !== "WEBGPU_DEVICE_LOST") {
          this.emit("fatalError", vistaError);
        }

        this.stop();
        return;
      }

      this.animationFrameId = window.requestAnimationFrame(tick);
    };

    this.animationFrameId = window.requestAnimationFrame(tick);
  }

  public stop(): void {
    this.running = false;

    if (this.animationFrameId !== null) {
      window.cancelAnimationFrame(this.animationFrameId);
      this.animationFrameId = null;
    }
  }

  public resize(width: number, height: number, devicePixelRatio: number = globalThis.devicePixelRatio ?? 1): void {
    // Checked before the canvas changes, so a bad size leaves it as it was.
    if (![width, height].every((side) => typeof side === "number" && side >= 0 && side <= MAX_CANVAS_SIDE)) {
      throw invalid(`resize() width and height must be from 0 to ${MAX_CANVAS_SIDE} CSS pixels, but they are ${width} and ${height}.`);
    }

    if (typeof devicePixelRatio !== "number" || !(devicePixelRatio > 0 && devicePixelRatio <= 8)) {
      throw invalid(`resize() devicePixelRatio must be over 0 and at most 8, but it is ${devicePixelRatio}.`);
    }

    if (this.pendingCall) {
      return;
    }

    this.call(() => {
      const nextWidth = Math.max(1, Math.floor(width));
      const nextHeight = Math.max(1, Math.floor(height));
      this.canvas.width = Math.max(1, Math.round(nextWidth * devicePixelRatio));
      this.canvas.height = Math.max(1, Math.round(nextHeight * devicePixelRatio));
      this.raw.resize(nextWidth, nextHeight, devicePixelRatio);
      this.snapshot.render = { width: nextWidth, height: nextHeight, devicePixelRatio };
    });
  }

  public dispose(): void {
    if (this.disposed) {
      return;
    }

    this.stop();
    this.disposed = true;
    this.listeners.clear();

    const disposeRaw = (): void => {
      // A trapped instance cannot be called safely; its memory goes with
      // the page.
      if (this.trap) {
        return;
      }

      try {
        this.raw.dispose();
      } catch (error) {
        // The engine is being torn down regardless; surface the failure
        // without throwing from a caller that has already moved on.
        console.error(toVistaWasmError(error));
      }
    };

    if (this.pendingCall) {
      this.pendingCall.then(disposeRaw, disposeRaw);
    } else {
      disposeRaw();
    }
  }

  public exportHeightmap(_options: ExportHeightmapOptions = {}): Uint8Array<ArrayBuffer> {
    if (this.pendingCall) {
      throw new VistaWasmError(
        "INTERNAL_ERROR",
        "Cannot export the heightmap while terrain is generating. Try again once generation completes."
      );
    }

    return this.call(() => this.raw.exportHeightmap());
  }

  public exportMap(kind: MapKind, options: ExportMapOptions = {}): ExportedMap {
    if (!MAP_KINDS.includes(kind)) {
      throw new TypeError(`exportMap() kind must be one of: ${MAP_KINDS.join(", ")}.`);
    }

    checkOptions("exportMap()", options);
    const size = options.size === undefined ? undefined : checkSize("exportMap()", options.size);
    const map = this.callWhenExporting<RawExportedMap>(() =>
      this.raw.exportMap(MAP_KINDS.indexOf(kind), size?.[0], size?.[1])
    );
    // Colours are stored as f32; six figures give back the decimals.
    const colour = (at: number) => Number(map.legendColours[at].toPrecision(6));
    const legend = map.legendNames
      ? map.legendNames.split("\n").map((name, index) => ({
        index,
        name,
        colour: [colour(index * 3), colour(index * 3 + 1), colour(index * 3 + 2)] as [number, number, number]
      }))
      : undefined;
    return {
      kind,
      width: map.width,
      height: map.height,
      channels: map.channels,
      type: map.data instanceof Float32Array ? "float32" : "uint8",
      data: map.data,
      encoding: {
        units: map.units,
        scale: map.scale,
        range: map.range && [map.range[0], map.range[1]],
        legend,
        metresPerPixel: [map.metresPerPixel[0], map.metresPerPixel[1]],
        seaLevelMetres: map.seaLevelMetres,
        generator: map.generator
      }
    };
  }

  public exportTrees(options: ExportTreesOptions = {}): TreeRecord[] {
    const { region, maxCount = 2_000_000 } = checkOptions<ExportTreesOptions>("exportTrees()", options);
    let bounds: Float32Array | undefined;

    if (region !== undefined) {
      const values = [region?.minX, region?.minZ, region?.maxX, region?.maxZ];

      if (!values.every((value) => typeof value === "number" && Number.isFinite(value))) {
        throw new TypeError("exportTrees() region must hold finite minX, minZ, maxX and maxZ in metres.");
      }

      if (region.minX > region.maxX || region.minZ > region.maxZ) {
        throw invalid(
          `exportTrees() region must have minX <= maxX and minZ <= maxZ, but it spans x ${region.minX} to ${region.maxX} and z ${region.minZ} to ${region.maxZ}.`
        );
      }

      bounds = Float32Array.from(values as number[]);
    }

    if (!Number.isInteger(maxCount) || maxCount < 1 || maxCount > 10_000_000) {
      throw invalid(
        `exportTrees() maxCount must be a whole number from 1 to 10000000, but it is ${String(maxCount)}.`
      );
    }

    const packed = this.callWhenExporting<Float32Array>(() => this.raw.exportTrees(bounds, maxCount));
    const trees: TreeRecord[] = [];

    for (let at = 0; at + 10 <= packed.length; at += 10) {
      const tree: TreeRecord = {
        x: packed[at],
        y: packed[at + 1],
        z: packed[at + 2],
        species: TREE_SPECIES[packed[at + 3]],
        variant: packed[at + 4],
        scale: packed[at + 5],
        rotation: packed[at + 6],
        tint: packed[at + 7],
        dryness: packed[at + 8]
      };

      if (packed[at + 9]) {
        tree.handPlaced = true;
      }

      trees.push(tree);
    }

    return trees;
  }

  public getOptionsSnapshot(): VistaOptionsSnapshot {
    this.ensureLive();
    // Undefined parts were never set: the engine uses their defaults.
    return JSON.parse(JSON.stringify(this.snapshot)) as VistaOptionsSnapshot;
  }

  public async exportSnapshot(options: SnapshotOptions = {}): Promise<Blob> {
    this.ensureLive();
    const { mimeType = "image/png", quality } = checkOptions<SnapshotOptions>("exportSnapshot()", options);

    if (typeof mimeType !== "string" || !/^image\/[\w.+-]{1,64}$/.test(mimeType)) {
      throw invalid(`exportSnapshot() mimeType must be an image type such as "image/png", but it is ${String(mimeType)}.`);
    }

    if (quality !== undefined && !(typeof quality === "number" && quality >= 0 && quality <= 1)) {
      throw invalid(`exportSnapshot() quality must be from 0 to 1, but it is ${String(quality)}.`);
    }

    return new Promise((resolve, reject) => {
      this.canvas.toBlob(
        (blob) => {
          if (!blob) {
            reject(new VistaWasmError("INTERNAL_ERROR", "Could not export a canvas snapshot."));
            return;
          }

          resolve(blob);
        },
        mimeType,
        quality
      );
    });
  }

  public on<EventName extends VistaEventName>(
    eventName: EventName,
    listener: VistaEventListener<EventName>
  ): () => void {
    if (!EVENT_NAMES.includes(eventName) || typeof listener !== "function") {
      throw new TypeError(`on() expects an event name, one of ${EVENT_NAMES.join(", ")}, and a listener function.`);
    }

    const listeners = this.listeners.get(eventName) ?? new Set();
    listeners.add(listener as (payload: unknown) => void);
    this.listeners.set(eventName, listeners);

    return () => {
      listeners.delete(listener as (payload: unknown) => void);
    };
  }

  public emit<EventName extends VistaEventName>(
    eventName: EventName,
    payload: VistaEventMap[EventName]
  ): void {
    const listeners = this.listeners.get(eventName);

    if (!listeners) {
      return;
    }

    for (const listener of listeners) {
      listener(payload);
    }
  }

  /** Emit a `"warning"` event for each line of `lines`. */
  public warn(lines: unknown): void {
    if (typeof lines === "string") {
      lines.split("\n").filter(Boolean).forEach((message) => this.emit("warning", { message }));
    }
  }

  /**
   * Check settings as the setters would, without applying them, for
   * `loadBundle()`: an error here leaves the scene as it was.
   */
  public checkSettings(options: VistaEngineOptions): void {
    const checked = normaliseEngineOptions(options, []);
    this.call(() => this.raw.checkOptions(checked));
  }

  /**
   * A checked copy of a setter's options, without the keys the engine
   * does not read: each is reported as a `"warning"` event.
   */
  private options<Options>(call: string, value: Options, keys: OptionKeys): Options {
    const warnings: string[] = [];
    const options = cloneOptions<Options>(call, value, keys, warnings);
    this.warn(warnings.join("\n"));
    return options;
  }

  private emitWarnings(handle: TerrainHandle): void {
    for (const message of handle.metadata.warnings) {
      this.emit("warning", {
        message
      });
    }
  }

  /**
   * Hooks change engine state that must not be dropped silently, so unlike
   * per-frame setters they fail loudly while terrain is generating.
   */
  private callWhenIdle(fn: () => unknown): void {
    if (this.pendingCall) {
      throw new VistaWasmError(
        "INTERNAL_ERROR",
        "Cannot replace trees, textures or painted maps, skip the weather ahead or set the time of day while terrain is generating. Await the terrain call first."
      );
    }

    this.call(fn);
  }

  /** Exports read the whole terrain, which generation is replacing. */
  private callWhenExporting<Result>(fn: () => unknown): Result {
    if (this.pendingCall) {
      throw new VistaWasmError(
        "INTERNAL_ERROR",
        "Cannot export maps or trees while terrain is generating. Try again once generation completes."
      );
    }

    return this.call(fn);
  }

  private call<Result>(fn: () => unknown): Result {
    this.ensureLive();

    try {
      return fn() as Result;
    } catch (error) {
      throw this.failure(error);
    }
  }

  /**
   * Emit `"gpuError"` for each GPU error no error scope caught, and
   * `"deviceLost"` if the browser lost the device, with its reason.
   */
  private reportGpuEvents(): void {
    const events = this.raw.takeGpuEvents?.();

    if (typeof events !== "string") {
      return;
    }

    const [lost, ...errors] = events.split("\n");

    for (const message of errors) {
      this.emit("gpuError", new VistaWasmError("GPU_ERROR", `The GPU reported an error: ${message}`));
    }

    if (lost) {
      this.loseDevice(new VistaWasmError("WEBGPU_DEVICE_LOST", `The WebGPU device was lost: ${lost.replace(/\.+$/, "")}. Dispose of this engine and create a new one to continue.`));
    }
  }

  /** Stop rendering and emit `"deviceLost"`, once. */
  private loseDevice(error: VistaWasmError): void {
    this.stop();

    if (!this.lost) {
      this.lost = true;
      this.emit("deviceLost", error);
    }
  }

  /**
   * The error to throw for `error`. A WebAssembly trap stops the engine
   * for good: it is marked dead and `"fatalError"` is emitted, once.
   */
  private failure(error: unknown): VistaWasmError {
    if (error instanceof WebAssembly.RuntimeError && !this.trap) {
      this.trap = error;
      this.stop();
      const fatal = new VistaWasmError(
        "INTERNAL_ERROR",
        `The VistaWASM engine stopped after an internal error (${error.message}). Create a new engine to continue.`,
        error
      );
      this.emit("fatalError", fatal);
      return fatal;
    }

    const vistaError = toVistaWasmError(error);

    // The browser's reason, when it has been reported, makes the better
    // event; rendering stops either way.
    if (vistaError.code === "WEBGPU_DEVICE_LOST") {
      this.reportGpuEvents();
      this.loseDevice(vistaError);
    }

    return vistaError;
  }

  private async callAsync<Result>(fn: () => Promise<unknown>): Promise<Result> {
    this.ensureLive();

    const previous = this.pendingCall ?? Promise.resolve();
    const call = previous.catch(() => undefined).then(async () => {
      this.ensureLive();
      // A trap in an async call surfaces as an uncaught error from the
      // task that polled it, and its promise never settles; the page's
      // error event ends the wait instead.
      let onError: ((event: ErrorEvent) => void) | undefined;
      const trapped = new Promise<never>((_, reject) => {
        onError = (event) => event.error instanceof WebAssembly.RuntimeError && reject(event.error);
        globalThis.addEventListener?.("error", onError);
      });

      try {
        return await Promise.race([fn(), trapped]);
      } catch (error) {
        throw this.failure(error);
      } finally {
        globalThis.removeEventListener?.("error", onError as (event: ErrorEvent) => void);

        if (!this.trap) {
          this.reportGpuEvents();
        }
      }
    });
    this.pendingCall = call;

    try {
      return (await call) as Result;
    } finally {
      if (this.pendingCall === call) {
        this.pendingCall = null;
      }
    }
  }

  private ensureLive(): void {
    if (this.disposed) {
      throw new VistaWasmError("ENGINE_DISPOSED", "The VistaWASM engine has been disposed.");
    }

    if (this.trap) {
      throw new VistaWasmError(
        "ENGINE_DISPOSED",
        `The VistaWASM engine stopped after an internal error (${this.trap.message}) and must be recreated: dispose of it and call createVistaEngine() again.`,
        this.trap
      );
    }
  }
}

/**
 * Load the generated module: `options.wasmModule`, or the glue at
 * `options.moduleUrl`, by default the copy next to this file. The glue
 * fetches the WASM from `options.wasmUrl`. Without one it is the copy
 * next to this file when the glue is too, and otherwise the file next to
 * the glue, so nothing loads from elsewhere unless the caller says so.
 */
async function loadWasmModule(options: VistaWasmInitOptions): Promise<VistaWasmGeneratedModule> {
  const moduleUrl = options.moduleUrl ?? new URL("./pkg/vista_wasm.js", import.meta.url);
  // Bundlers copy the glue as a plain asset without following the WASM
  // reference inside it, so the default glue is handed the WASM's URL
  // from here, where a bundler sees it and emits the file.
  const wasmUrl = options.wasmUrl
    ?? (options.wasmModule || options.moduleUrl ? undefined : new URL("./pkg/vista_wasm_bg.wasm", import.meta.url));

  try {
    const module = options.wasmModule ?? await importGlue(moduleUrl);
    await module.default?.(wasmUrl === undefined ? undefined : { module_or_path: wasmUrl });
    module.initialiseVistaWasm?.();
    return module;
  } catch (error) {
    throw new VistaWasmError(
      "WASM_LOAD_FAILED",
      `VistaWASM could not load its ${options.wasmModule ? "" : `module (${String(moduleUrl)}) or its `}WASM: ${(error as Error)?.message ?? String(error)}. Check that both are served, the .wasm file as application/wasm, or pass wasmUrl or moduleUrl.`,
      error
    );
  }
}

function assertCanvas(canvas: HTMLCanvasElement): void {
  const canvasConstructor = globalThis.HTMLCanvasElement;

  if (!canvas || (typeof canvasConstructor === "function" && !(canvas instanceof canvasConstructor))) {
    throw new VistaWasmError(
      "CANVAS_INVALID",
      "createVistaEngine() requires an HTMLCanvasElement supplied by the host frontend."
    );
  }
}

function normaliseEngineOptions(options: VistaEngineOptions, warnings: string[]): VistaEngineOptions {
  options = cloneOptions("createVistaEngine()", options, OPTION_KEYS.engine, warnings);
  return {
    ...options,
    flora: options.flora ? normaliseFloraOptions(options.flora) : undefined,
    grass: options.grass ? normaliseGrassOptions(options.grass) : undefined,
    clouds: options.clouds ? normaliseCloudsOptions(options.clouds) : undefined,
    mist: options.mist ? normaliseMistOptions(options.mist) : undefined,
    biomes: options.biomes ? normaliseBiomeOptions(options.biomes) : undefined,
    weather: options.weather && normaliseWeather("createVistaEngine() weather", options.weather)
  };
}

/**
 * Check weather options, and custom presets above all: the presets are a
 * map from name to preset, so they must be a plain object, with a bounded
 * number of names of bounded length.
 */
function normaliseWeather(call: string, weather: WeatherOptions, warnings?: string[]): WeatherOptions {
  // A clone keeps only own keys, so the presets' prototype is checked on
  // the caller's object, without running a getter.
  const given = typeof weather === "object" && weather !== null ? Object.getOwnPropertyDescriptor(weather, "presets")?.value : undefined;
  weather = cloneOptions<WeatherOptions>(call, weather, warnings && OPTION_KEYS.engine.weather, warnings);
  const { presets, state, seedOffset } = weather;
  const names = presets === undefined ? [] : Object.keys(presets);
  const prototype = typeof given === "object" && given !== null && Object.getPrototypeOf(given);

  if (presets !== undefined && (typeof presets !== "object" || presets === null || Array.isArray(presets) || (given !== undefined && prototype !== Object.prototype && prototype !== null))) {
    throw new TypeError(`${call} presets must be a plain object of presets by name.`);
  }

  if (names.length > MAX_WEATHER_PRESETS) {
    throw invalid(`${call} presets may hold at most ${MAX_WEATHER_PRESETS} presets, but it holds ${names.length}.`);
  }

  for (const name of [...names, ...(state === undefined ? [] : [state])]) {
    if (typeof name !== "string" || name.length < 1 || name.length > MAX_PRESET_NAME_LENGTH) {
      throw invalid(`${call} preset names and state must be strings of 1 to ${MAX_PRESET_NAME_LENGTH} characters, but one is ${JSON.stringify(name)?.slice(0, 80)}.`);
    }
  }

  return seedOffset === undefined ? { ...weather } : { ...weather, seedOffset: normaliseInteger(seedOffset) };
}

/** Terrain arrives as an `ArrayBuffer`; anything else would reach the engine as garbage. */
function checkBuffer(call: string, buffer: unknown): void {
  if (!(buffer instanceof ArrayBuffer)) {
    throw new TypeError(`${call} expects the file's bytes as an ArrayBuffer (use typedArray.slice().buffer for a typed array).`);
  }
}

/**
 * Check a raw heightmap buffer's length against what its options need,
 * before it reaches the engine: shorter is an error, and so is longer
 * than `MAX_RAW_HEIGHTMAP_BYTES`, which no map needs. A buffer
 * that is merely longer than needed is read in part, with a warning from
 * the engine. Options it cannot size are left for the engine to reject.
 */
function checkRawLength(buffer: ArrayBuffer, options: RawHeightmapOptions): void {
  const { width, height, sampleFormat } = options;
  const sample = SAMPLE_BYTES[sampleFormat as string];
  const sized = sample && [width, height].every((side) => Number.isInteger(side) && side >= 2 && side <= MAX_TERRAIN_SIDE);
  const needed = sized ? width * height * sample : 0;

  if (sized && buffer.byteLength < needed) {
    throw invalid(`loadRawHeightmap() was given ${buffer.byteLength} bytes, but ${width} x ${height} ${sampleFormat} samples need ${needed}.`);
  }

  if (buffer.byteLength > MAX_RAW_HEIGHTMAP_BYTES) {
    throw invalid(`loadRawHeightmap() was given ${buffer.byteLength} bytes, more than the ${MAX_RAW_HEIGHTMAP_BYTES} the largest map (${MAX_TERRAIN_SIDE} x ${MAX_TERRAIN_SIDE} float32 samples) needs${sized ? `; ${width} x ${height} ${sampleFormat} samples need ${needed}` : ""}.`);
  }
}

/**
 * Check a painted map's shape: whole-number sizes from 2 to
 * `MAX_TERRAIN_SIDE` (2048) and a `Uint8Array` of width x height bytes.
 */
function checkMask(name: string, mask: BiomeMap | DensityMask): void {
  if (
    !mask ||
    typeof mask !== "object" ||
    !(mask.data instanceof Uint8Array) ||
    !Number.isInteger(mask.width) ||
    !Number.isInteger(mask.height)
  ) {
    throw new TypeError(`${name} expects { width, height, data } with whole-number sizes and a Uint8Array, or null.`);
  }

  if (mask.width < 2 || mask.width > MAX_TERRAIN_SIDE || mask.height < 2 || mask.height > MAX_TERRAIN_SIDE) {
    throw invalid(`${name} width and height must be from 2 to ${MAX_TERRAIN_SIDE} (${TERRAIN_SIZE_REASON}), but they are ${mask.width} and ${mask.height}.`);
  }

  if (mask.data.length !== mask.width * mask.height) {
    throw invalid(`${name} data must hold width x height = ${mask.width * mask.height} bytes, but it holds ${mask.data.length}.`);
  }
}

/** A painted map with its own copy of the data, so later edits by the caller do not change it. */
function copyMask<Mask extends { width: number; height: number; data: Uint8Array }>(mask: Mask) {
  return { width: mask.width, height: mask.height, data: mask.data.slice() };
}

function assertSpecies(species: TreeSpecies): void {
  if (!TREE_SPECIES.includes(species)) {
    throw new TypeError(`Unknown tree species. Use one of: ${TREE_SPECIES.join(", ")}.`);
  }
}

function toFloat32(name: string, values: Float32Array | number[]): Float32Array {
  if (values instanceof Float32Array) {
    return values;
  }

  if (!Array.isArray(values) || !values.every((value) => typeof value === "number")) {
    throw new TypeError(`Tree model ${name} must be a Float32Array or an array of numbers.`);
  }

  return Float32Array.from(values);
}

function toUint32(name: string, values: Uint32Array | number[]): Uint32Array {
  if (values instanceof Uint32Array) {
    return values;
  }

  // A Uint32Array would wrap negative, fractional or larger values round.
  if (!Array.isArray(values) || !values.every((value) => Number.isInteger(value) && value >= 0 && value <= 0xffffffff)) {
    throw new TypeError(`Tree model ${name} must be a Uint32Array or an array of whole numbers from 0 to 4294967295.`);
  }

  return Uint32Array.from(values);
}

function finite(call: string, name: string, value: unknown, fallback?: number): number {
  if (value === undefined && fallback !== undefined) {
    return fallback;
  }

  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw invalid(`${call} ${name} must be a finite number of metres, but it is ${String(value)}.`);
  }

  return value;
}

/**
 * Decode a heightmap image into float32 heights and the
 * `loadRawHeightmap()` options that load them.
 */
async function heightmapFromImage(
  source: ImageSource,
  options: HeightmapImageImportOptions,
  warn: (message: string) => void
): Promise<{ buffer: ArrayBuffer; raw: RawHeightmapOptions }> {
  const call = "loadHeightmapImage()";
  const { metresPerSample, minHeightMetres, maxHeightMetres, seaLevelMetres, channel } = checkOptions<HeightmapImageImportOptions>(call, options);
  const metres = finite(call, "metresPerSample", metresPerSample);
  const sea = finite(call, "seaLevelMetres", seaLevelMetres, 0);

  if (metres <= 0) {
    throw invalid(`${call} metresPerSample must be over 0, but it is ${metres}.`);
  }

  // A heightmap image becomes the terrain, so it is held to the terrain's
  // size, before it is decoded.
  const image = await decodeImage(source, "The heightmap image", warn, MAX_TERRAIN_SIDE, `heightmaps (${TERRAIN_SIZE_REASON})`);
  let range: unknown;

  try {
    range = JSON.parse(image.text["vistawasm:range"]);
  } catch {
    // No range stored: the options must give one.
  }

  const stored = Array.isArray(range) && range.length === 2 && range.every(Number.isFinite) && range[0] <= range[1];

  if (!stored && (minHeightMetres === undefined || maxHeightMetres === undefined)) {
    throw invalid(`${call} needs minHeightMetres and maxHeightMetres, as the image has no vistawasm:range text.`);
  }

  const low = finite(call, "minHeightMetres", minHeightMetres, stored ? (range as number[])[0] : undefined);
  const high = finite(call, "maxHeightMetres", maxHeightMetres, stored ? (range as number[])[1] : undefined);

  if (!(low < high || (stored && low === high))) {
    throw invalid(`${call} minHeightMetres (${low}) must be below maxHeightMetres (${high}).`);
  }

  const heights = channelOf(image, channel, call).map((value) => low + value * (high - low));
  return {
    buffer: heights.buffer,
    raw: {
      width: image.width,
      height: image.height,
      sampleFormat: "float32",
      byteOrder: "little-endian",
      metresPerSample: metres,
      heightScaleMetres: 1,
      seaLevelMetres: sea
    }
  };
}

/** Emit an engine event; engines without an emitter log warnings instead. */
function emitOn(engine: VistaEngine, name: "progress" | "warning", payload: object): void {
  const { emit } = engine as { emit?: (name: string, payload: object) => void };

  if (emit) {
    emit.call(engine, name, payload);
  } else if (name === "warning") {
    console.warn((payload as { message: string }).message);
  }
}

/** Apply painted maps: water, then biomes, then the density masks. */
function paint(engine: VistaEngine, maps: { water?: WaterMask; biome?: BiomeMap; trees?: DensityMask; grass?: DensityMask }): void {
  const { water, biome, trees, grass } = maps;

  if (water) {
    engine.setWaterMask(water);
  }

  if (biome) {
    engine.setBiomeMap(biome);
  }

  if (trees || grass) {
    engine.setVegetationMasks({ ...(trees && { trees }), ...(grass && { grass }) });
  }
}

/** Engine setters for each part of an options snapshot. */
const SETTERS = [
  ["sun", "setSun"],
  ["atmosphere", "setAtmosphere"],
  ["water", "setWater"],
  ["flora", "setFlora"],
  ["grass", "setGrass"],
  ["clouds", "setClouds"],
  ["mist", "setMist"],
  ["quality", "setRenderQuality"],
  ["biomes", "setBiomes"],
  ["weather", "setWeather"],
  ["shadows", "setShadows"],
  ["surface", "setSurface"]
] as const;

/**
 * Load a bundle from `exportBundle()`: the heights before any carving,
 * the options (unless `options.applySettings` is `false`) and the painted
 * maps. Everything else is recomputed from those, so a version 2 bundle
 * recreates the scene exactly; a version 1 bundle loads its final
 * heights, with a warning. Emits `"progress"` events with phase
 * `"bundle"`. See `docs/import.md`.
 */
export async function loadBundle(
  engine: VistaEngine,
  source: ImageSource,
  options: BundleLoadOptions = {}
): Promise<TerrainHandle> {
  const applySettings = checkOptions("loadBundle()", options).applySettings ?? true;

  if (typeof applySettings !== "boolean") {
    throw new TypeError("loadBundle() options must be { applySettings?: boolean }.");
  }

  const progress = (value: number) => emitOn(engine, "progress", { phase: "bundle", progress: value });
  progress(0);
  // Everything is read, and the settings checked, before the scene
  // changes, so a damaged bundle leaves it as it was.
  const bundle = await readBundle(source);
  const settings: [keyof VistaOptionsSnapshot, (typeof SETTERS)[number][1], unknown][] = [];

  for (const [key, setter] of applySettings ? SETTERS : []) {
    const value = bundle.options[key];

    if (value !== undefined) {
      if (typeof value !== "object" || value === null) {
        throw new VistaWasmError("INVALID_DEM", `The bundle's options.${key} must be an object.`);
      }

      settings.push([key, setter, value]);
    }
  }

  const camera = applySettings && typeof bundle.options.camera === "object" && bundle.options.camera ? bundle.options.camera as CameraOptions : undefined;
  // The engine checks the settings as its setters would, without applying
  // them; an engine without the check applies them as given.
  (engine as { checkSettings?: (options: VistaEngineOptions) => void }).checkSettings?.({
    ...Object.fromEntries(settings.map(([key, , value]) => [key, value])),
    ...(camera && { camera })
  });
  progress(0.3);
  const previous = engine.getOptionsSnapshot();
  const handle = await engine.loadRawHeightmap(bundle.heights, bundle.raw);
  progress(0.7);

  try {
    for (const [, setter, value] of settings) {
      (engine[setter] as (value: unknown) => void)(value);
    }
  } catch (error) {
    // Put back the settings the engine had: those it never had set keep
    // the bundle's.
    for (const [key, setter] of settings) {
      if (previous[key] !== undefined) {
        (engine[setter] as (value: unknown) => void)(previous[key]);
      }
    }

    throw error;
  }

  if (bundle.version === 1) {
    emitOn(engine, "warning", {
      message: "This bundle is version 1, which holds the final, carved heights: rivers, lakes and glaciers were shaped again on top of them. Export it again for an exact round trip."
    });
  }

  if (camera) {
    engine.setCamera(camera);
  }

  paint(engine, bundle);
  progress(1);
  return handle;
}

/**
 * Load a heightmap image with any painted maps in one call. Every image
 * is decoded first, so a bad one leaves the scene as it was; warnings
 * arrive as the engine's `"warning"` events.
 */
export async function loadTerrainFromImages(
  engine: VistaEngine,
  images: TerrainImages,
  options: HeightmapImageImportOptions
): Promise<TerrainHandle> {
  if (typeof images !== "object" || images?.height === undefined) {
    throw new TypeError("loadTerrainFromImages() expects images with at least a height image.");
  }

  const warnings: string[] = [];
  const onWarning = (message: string) => void warnings.push(message);
  const read = <Result>(image: ImageSource | undefined, reader: (image: ImageSource, options: { onWarning: typeof onWarning }) => Promise<Result>) =>
    image === undefined ? undefined : reader(image, { onWarning });
  const { buffer, raw } = await heightmapFromImage(images.height, options, onWarning);
  const maps = {
    water: await read(images.water, waterMaskFromImage),
    biome: (await read(images.biome, biomeMapFromImage))?.map,
    trees: await read(images.trees, densityMaskFromImage),
    grass: await read(images.grass, densityMaskFromImage)
  };
  const handle = await engine.loadRawHeightmap(buffer, raw);
  warnings.forEach((message) => emitOn(engine, "warning", { message }));
  paint(engine, maps);
  return handle;
}

/**
 * Draw an image into a 512 x 512 RGBA8 texture layer for
 * `replaceTexture()`. The image is stretched to fill the square, so use a
 * square, seamlessly tiling image.
 */
export async function imageToRgba(
  source: ImageBitmapSource,
  size = TEXTURE_LAYER_SIZE
): Promise<Uint8Array<ArrayBuffer>> {
  if (!Number.isInteger(size) || size < 1 || size > 8192) {
    throw invalid(`imageToRgba() size must be a whole number from 1 to 8192, but it is ${String(size)}.`);
  }

  const bitmap = await createImageBitmap(source, {
    resizeWidth: size,
    resizeHeight: size,
    resizeQuality: "high",
    premultiplyAlpha: "none",
    colorSpaceConversion: "none"
  });
  const canvas = new OffscreenCanvas(size, size);
  const context = canvas.getContext("2d");

  if (!context) {
    bitmap.close();
    throw new VistaWasmError("INTERNAL_ERROR", "Could not create a 2D canvas to read the image.");
  }

  context.drawImage(bitmap, 0, 0);
  bitmap.close();
  const pixels = context.getImageData(0, 0, size, size).data;
  return new Uint8Array(pixels.buffer.slice(0) as ArrayBuffer);
}

function normaliseFractalOptions(options: FractalTerrainOptions): FractalTerrainOptions {
  return {
    ...options,
    seed: normaliseInteger(options.seed)
  };
}

function normaliseFloraOptions(options: FloraOptions): FloraOptions {
  return withSeedOffset(options);
}

function normaliseGrassOptions(options: GrassOptions): GrassOptions {
  return withSeedOffset(options);
}

function normaliseCloudsOptions(options: CloudsOptions): CloudsOptions {
  return withSeedOffset(options);
}

function normaliseMistOptions(options: MistOptions): MistOptions {
  return withSeedOffset(options);
}

function normaliseBiomeOptions(options: BiomeOptions): BiomeOptions {
  return withSeedOffset(options);
}

/**
 * A copy with `seedOffset` as a number, when it is given. A key set to
 * `undefined` would reach the engine as a value that is not a number.
 */
function withSeedOffset<Options extends { seedOffset?: number | bigint }>(options: Options): Options {
  return options.seedOffset === undefined
    ? { ...options }
    : { ...options, seedOffset: normaliseInteger(options.seedOffset) };
}

function normaliseInteger(value: number | bigint): number {
  if (typeof value === "bigint") {
    return Number(value & BigInt("0xffffffffffffffff"));
  }

  return value;
}
