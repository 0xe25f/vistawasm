import { VistaWasmError, toVistaWasmError } from "./errors.js";
import { assertVistaWasmSupport, detectVistaWasmSupport } from "./feature-detect.js";
import type {
  AtmosphereOptions,
  BiomeKind,
  BiomeOptions,
  CameraOptions,
  CloudsOptions,
  DebugView,
  DemFetchOptions,
  DemLoadOptions,
  ExportHeightmapOptions,
  FloraOptions,
  FractalTerrainOptions,
  GrassOptions,
  MistOptions,
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
  TreeSpecies,
  VistaEngine,
  VistaEngineOptions,
  VistaEventListener,
  VistaEventMap,
  VistaEventName,
  VistaWasmGeneratedModule,
  VistaWasmInitOptions,
  VistaWasmRawEngine,
  WaterOptions,
  WeatherKind,
  WeatherOptions,
  WeatherState
} from "./types.js";

const TREE_SPECIES: readonly TreeSpecies[] = [
  "oak",
  "pine",
  "spruce",
  "palm",
  "jungle",
  "cypress",
  "acacia",
  "shrub"
];

/** Edge length, in texels, of every replaceable texture layer. */
export const TEXTURE_LAYER_SIZE = 512;


export { VistaWasmError, detectVistaWasmSupport };
export type * from "./types.js";
export * from "./camera-controls.js";
export * from "./terrain-export.js";

let loadedModule: VistaWasmGeneratedModule | null = null;
let loadingModule: Promise<VistaWasmGeneratedModule> | null = null;

/**
 * Load the generated WASM module once.
 */
export async function initialiseVistaWasm(
  options: VistaWasmInitOptions = {}
): Promise<void> {
  if (loadedModule) {
    return;
  }

  loadingModule ??= loadWasmModule(options);
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
  await initialiseVistaWasm();

  if (!loadedModule) {
    throw new VistaWasmError("INTERNAL_ERROR", "VistaWASM failed to load its WASM module.");
  }

  try {
    const raw = await loadedModule.VistaEngine.create(canvas, normaliseEngineOptions(options));
    const engine = new VistaEngineWrapper(canvas, raw);
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
  const response = await fetch(url, {
    headers: options.headers,
    signal: options.signal
  });

  if (!response.ok) {
    throw new VistaWasmError(
      "DEM_FETCH_FAILED",
      `Could not fetch DEM data. HTTP ${response.status}.`
    );
  }

  return response.arrayBuffer();
}

class VistaEngineWrapper implements VistaEngine {
  private readonly listeners = new Map<VistaEventName, Set<(payload: unknown) => void>>();

  private animationFrameId: number | null = null;

  private running = false;

  private disposed = false;

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

  private lastStats: RenderStats = {
    frameIndex: 0,
    frameTimeMs: 0,
    terrainTriangles: 0,
    floraInstances: 0,
    grassInstances: 0,
    clipmapLevels: 0
  };

  public constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly raw: VistaWasmRawEngine
  ) {}

  public async generateFractal(options: FractalTerrainOptions): Promise<TerrainHandle> {
    this.ensureLive();
    this.emit("progress", {
      phase: "fractal",
      progress: 0
    });
    const handle = await this.callAsync<TerrainHandle>(() =>
      this.raw.generateFractal(normaliseFractalOptions(options))
    );
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
    const buffer = await fetchDemBytes(url, options);
    return this.loadDemFromArrayBuffer(buffer, options);
  }

  public async loadDemFromArrayBuffer(
    buffer: ArrayBuffer,
    options: DemLoadOptions = {}
  ): Promise<TerrainHandle> {
    this.ensureLive();
    const handle = await this.callAsync<TerrainHandle>(() =>
      this.raw.loadDemFromArrayBuffer(buffer, options)
    );
    this.emitWarnings(handle);
    this.emit("terrainLoaded", handle);
    return handle;
  }

  public async loadRawHeightmap(
    buffer: ArrayBuffer,
    options: RawHeightmapOptions
  ): Promise<TerrainHandle> {
    this.ensureLive();
    const handle = await this.callAsync<TerrainHandle>(() =>
      this.raw.loadRawHeightmap(buffer, options)
    );
    this.emitWarnings(handle);
    this.emit("terrainLoaded", handle);
    return handle;
  }

  public setCamera(camera: CameraOptions): void {
    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setCamera(camera));
  }

  public setSun(sun: SunOptions): void {
    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setSun(sun));
  }

  public setAtmosphere(atmosphere: AtmosphereOptions): void {
    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setAtmosphere(atmosphere));
  }

  public setWater(water: WaterOptions): void {
    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setWater(water));
  }

  public setFlora(flora: FloraOptions): void {
    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setFlora(normaliseFloraOptions(flora)));
  }

  public setGrass(grass: GrassOptions): void {
    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setGrass(normaliseGrassOptions(grass)));
  }

  public setClouds(clouds: CloudsOptions): void {
    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setClouds(normaliseCloudsOptions(clouds)));
  }

  public setMist(mist: MistOptions): void {
    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setMist(normaliseMistOptions(mist)));
  }

  public setRenderQuality(quality: RenderQualityOptions): void {
    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setRenderQuality(quality));
  }

  public setBiomes(biomes: BiomeOptions): void {
    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setBiomes(normaliseBiomeOptions(biomes)));
  }

  public biomeAt(x: number, z: number): BiomeKind | undefined {
    if (this.pendingCall || !Number.isFinite(x) || !Number.isFinite(z)) {
      return undefined;
    }

    return this.call(() => this.raw.biomeAt(x, z)) as BiomeKind | undefined;
  }

  public setDebugView(debugView: DebugView): void {
    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setDebugView(debugView));
  }

  public setWeather(weather: WeatherOptions): void {
    if (this.pendingCall) {
      return;
    }

    const normalised =
      weather.seedOffset === undefined
        ? { ...weather }
        : { ...weather, seedOffset: normaliseInteger(weather.seedOffset) };
    this.call(() => this.raw.setWeather(normalised));
  }

  public getWeather(): WeatherState | undefined {
    if (this.pendingCall) {
      return undefined;
    }

    return this.call<WeatherState | undefined>(() => this.raw.getWeather());
  }

  public setShadows(shadows: ShadowOptions): void {
    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setShadows(shadows));
  }

  public setSurface(surface: SurfaceOptions): void {
    if (this.pendingCall) {
      return;
    }

    this.call(() => this.raw.setSurface(surface));
  }

  public setTreeModel(species: TreeSpecies, model: TreeModel): void {
    assertSpecies(species);

    if (!model || typeof model !== "object") {
      throw new TypeError("setTreeModel() expects a model with positions, normals, uvs, and indices.");
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

    // Eight floats per tree keeps this one boundary crossing, however many
    // trees there are.
    const packed = new Float32Array(trees.length * 8);

    trees.forEach((tree, index) => {
      const species = TREE_SPECIES.indexOf(tree.species);

      if (species < 0) {
        throw new TypeError(
          `Tree ${index} has an unknown species. Use one of: ${TREE_SPECIES.join(", ")}.`
        );
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
          tree.dryness ?? 0
        ],
        index * 8
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

    if (!Number.isInteger(layer) || layer < 0) {
      throw new TypeError("replaceTexture() layer must be a whole number of at least 0.");
    }

    if (!(rgba instanceof Uint8Array || rgba instanceof Uint8ClampedArray)) {
      throw new TypeError("replaceTexture() expects RGBA texels in a Uint8Array.");
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

    // An unchanged frame index means the GPU was still busy with earlier
    // frames, so the engine skipped this one rather than queue it.
    if (stats.frameIndex === this.lastStats.frameIndex) {
      return this.lastStats;
    }

    stats.frameTimeMs = performance.now() - start;
    stats.weather ??= null;
    const weatherChanged = stats.weather !== this.lastWeather;
    this.lastWeather = stats.weather;
    this.lastStats = stats;
    this.emit("stats", stats);

    if (weatherChanged) {
      this.emit("weatherChanged", stats.weather);
    }

    return stats;
  }

  public start(): void {
    this.ensureLive();

    if (this.running) {
      return;
    }

    this.running = true;
    const tick = () => {
      if (!this.running) {
        return;
      }

      try {
        this.renderOnce();
      } catch (error) {
        const vistaError = toVistaWasmError(error);
        this.emit(vistaError.code === "WEBGPU_DEVICE_LOST" ? "deviceLost" : "fatalError", vistaError);
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

  public resize(width: number, height: number, devicePixelRatio = window.devicePixelRatio): void {
    if (this.pendingCall) {
      return;
    }

    this.call(() => {
      const pixelRatio = Number.isFinite(devicePixelRatio) ? devicePixelRatio : 1;
      const nextWidth = Math.max(1, Math.floor(width));
      const nextHeight = Math.max(1, Math.floor(height));
      this.canvas.width = Math.max(1, Math.round(nextWidth * pixelRatio));
      this.canvas.height = Math.max(1, Math.round(nextHeight * pixelRatio));
      this.raw.resize(nextWidth, nextHeight, pixelRatio);
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

  public async exportSnapshot(options: SnapshotOptions = {}): Promise<Blob> {
    this.ensureLive();
    const mimeType = options.mimeType ?? "image/png";

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
        options.quality
      );
    });
  }

  public on<EventName extends VistaEventName>(
    eventName: EventName,
    listener: VistaEventListener<EventName>
  ): () => void {
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
        "Cannot replace trees or textures while terrain is generating. Await the terrain call first."
      );
    }

    this.call(fn);
  }

  private call<Result>(fn: () => unknown): Result {
    this.ensureLive();

    try {
      return fn() as Result;
    } catch (error) {
      throw toVistaWasmError(error);
    }
  }

  private async callAsync<Result>(fn: () => Promise<unknown>): Promise<Result> {
    this.ensureLive();

    const previous = this.pendingCall ?? Promise.resolve();
    const call = previous.catch(() => undefined).then(async () => {
      this.ensureLive();

      try {
        return await fn();
      } catch (error) {
        throw toVistaWasmError(error);
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
  }
}

async function loadWasmModule(options: VistaWasmInitOptions): Promise<VistaWasmGeneratedModule> {
  if (options.wasmModule) {
    await options.wasmModule.default?.(options.wasmUrl);
    options.wasmModule.initialiseVistaWasm?.();
    return options.wasmModule;
  }

  const moduleUrl = options.moduleUrl ?? new URL("./pkg/vista_wasm.js", import.meta.url);
  const module = (await import(
    /* @vite-ignore */ moduleUrl.toString()
  )) as VistaWasmGeneratedModule;

  await module.default?.(options.wasmUrl);
  module.initialiseVistaWasm?.();
  return module;
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

function normaliseEngineOptions(options: VistaEngineOptions): VistaEngineOptions {
  return {
    ...options,
    flora: options.flora ? normaliseFloraOptions(options.flora) : undefined,
    grass: options.grass ? normaliseGrassOptions(options.grass) : undefined,
    clouds: options.clouds ? normaliseCloudsOptions(options.clouds) : undefined,
    mist: options.mist ? normaliseMistOptions(options.mist) : undefined,
    biomes: options.biomes ? normaliseBiomeOptions(options.biomes) : undefined,
    weather:
      options.weather?.seedOffset === undefined
        ? options.weather
        : { ...options.weather, seedOffset: normaliseInteger(options.weather.seedOffset) }
  };
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

  if (!Array.isArray(values)) {
    throw new TypeError(`Tree model ${name} must be a Float32Array or an array of numbers.`);
  }

  return Float32Array.from(values);
}

function toUint32(name: string, values: Uint32Array | number[]): Uint32Array {
  if (values instanceof Uint32Array) {
    return values;
  }

  if (!Array.isArray(values)) {
    throw new TypeError(`Tree model ${name} must be a Uint32Array or an array of numbers.`);
  }

  return Uint32Array.from(values);
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
  return {
    ...options,
    seedOffset: normaliseInteger(options.seedOffset)
  };
}

function normaliseGrassOptions(options: GrassOptions): GrassOptions {
  return {
    ...options,
    seedOffset: normaliseInteger(options.seedOffset)
  };
}

function normaliseCloudsOptions(options: CloudsOptions): CloudsOptions {
  return {
    ...options,
    seedOffset: normaliseInteger(options.seedOffset)
  };
}

function normaliseMistOptions(options: MistOptions): MistOptions {
  return {
    ...options,
    seedOffset: normaliseInteger(options.seedOffset)
  };
}

function normaliseBiomeOptions(options: BiomeOptions): BiomeOptions {
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
