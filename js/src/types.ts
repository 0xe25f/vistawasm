/**
 * Stable VistaWASM error codes.
 */
export type VistaErrorCode =
  | "WEBGPU_UNAVAILABLE"
  | "WEBGPU_DEVICE_REQUEST_FAILED"
  | "WEBGPU_DEVICE_LOST"
  | "CANVAS_INVALID"
  | "OPTIONS_INVALID"
  | "TERRAIN_GENERATION_FAILED"
  | "DEM_FETCH_FAILED"
  | "DEM_FORMAT_UNSUPPORTED"
  | "DEM_METADATA_MISSING"
  | "GPU_LIMIT_EXCEEDED"
  | "ENGINE_DISPOSED"
  | "INTERNAL_ERROR";

/**
 * Module override for tests or custom WASM hosting.
 */
export interface VistaWasmInitOptions {
  /**
   * Explicit WASM URL or compiled module passed to the generated loader.
   */
  wasmUrl?: string | URL | Request | Response | BufferSource | WebAssembly.Module;

  /**
   * Explicit generated `wasm-pack` module.
   */
  wasmModule?: VistaWasmGeneratedModule;

  /**
   * Explicit generated JavaScript module URL.
   */
  moduleUrl?: string | URL;
}

/**
 * Minimal shape of the generated `wasm-pack` module.
 */
export interface VistaWasmGeneratedModule {
  default?: (input?: unknown) => Promise<unknown> | unknown;
  initialiseVistaWasm?: () => void;
  VistaEngine: VistaWasmRawEngineConstructor;
}

/**
 * Generated Rust engine constructor.
 */
export interface VistaWasmRawEngineConstructor {
  create(canvas: HTMLCanvasElement, options?: unknown): Promise<VistaWasmRawEngine>;
}

/**
 * Generated Rust engine instance.
 */
export interface VistaWasmRawEngine {
  generateFractal(options: unknown): Promise<unknown>;
  loadDemFromArrayBuffer(buffer: ArrayBuffer, options?: unknown): Promise<unknown>;
  loadRawHeightmap(buffer: ArrayBuffer, options: unknown): Promise<unknown>;
  setCamera(camera: unknown): void;
  setSun(sun: unknown): void;
  setAtmosphere(atmosphere: unknown): void;
  setWater(water: unknown): void;
  setFlora(flora: unknown): void;
  setGrass(grass: unknown): void;
  setClouds(clouds: unknown): void;
  setMist(mist: unknown): void;
  setRenderQuality(quality: unknown): void;
  setDebugView(debugView: unknown): void;
  renderOnce(): unknown;
  resize(width: number, height: number, devicePixelRatio?: number): void;
  dispose(): void;
  exportHeightmap(): Uint8Array<ArrayBuffer>;
  getStats(): unknown;
}

/**
 * Public engine options.
 */
export interface VistaEngineOptions {
  render?: RenderSizeOptions;
  camera?: CameraOptions;
  sun?: SunOptions;
  atmosphere?: AtmosphereOptions;
  water?: WaterOptions;
  flora?: FloraOptions;
  grass?: GrassOptions;
  clouds?: CloudsOptions;
  mist?: MistOptions;
  quality?: RenderQualityOptions;
}

/**
 * Render size supplied by the host.
 */
export interface RenderSizeOptions {
  width: number;
  height: number;
  devicePixelRatio?: number;
}

/**
 * Terrain handle returned by generation and loading jobs.
 */
export interface TerrainHandle {
  id: number;
  metadata: TerrainMetadata;
}

/**
 * Terrain metadata exposed to host applications.
 */
export interface TerrainMetadata {
  width: number;
  height: number;
  metresPerSample: number;
  verticalScale: number;
  seaLevelMetres: number;
  minHeightMetres: number;
  maxHeightMetres: number;
  meanHeightMetres: number;
  source: string;
  generatorVersion: string;
  geospatial?: GeospatialMetadata | null;
  warnings: string[];
}

/**
 * Geospatial metadata decoded from DEM data.
 */
export interface GeospatialMetadata {
  metresPerSampleX?: number | null;
  metresPerSampleY?: number | null;
  projectionName?: string | null;
  modelTiepoint?: [number, number, number, number, number, number] | null;
  modelPixelScale?: [number, number, number] | null;
}

/**
 * Fractal terrain generation options.
 */
export interface FractalTerrainOptions {
  seed: number | bigint;
  size: 512 | 1024 | 2048 | 4096 | 8192 | number;
  horizontalScaleMetres: number;
  verticalScale: number;
  baseHeightMetres?: number;
  seaLevelMetres?: number;
  noise: NoiseOptions;
  shape?: TerrainShapeOptions;
  erosion?: ErosionOptions;
}

/**
 * Supported procedural noise names.
 */
export type NoiseKind =
  | "simplex"
  | "ridged"
  | "hybrid"
  | "island"
  | "canyon"
  | "cratered"
  | "classic";

/**
 * Fractal noise controls.
 */
export interface NoiseOptions {
  kind: NoiseKind;
  octaves: number;
  gain: number;
  lacunarity: number;
  warp?: number;
}

/**
 * Large-scale terrain shape controls.
 */
export interface TerrainShapeOptions {
  island?: number;
  terrace?: number;
  basin?: number;
  canyon?: number;
  crater?: number;
}

/**
 * Erosion quality preset.
 */
export type ErosionQuality = "preview" | "balanced" | "high" | "offline";

/**
 * Erosion controls.
 */
export interface ErosionOptions {
  hydraulicIterations?: number;
  thermalIterations?: number;
  rainAmount?: number;
  evaporation?: number;
  sedimentCapacity?: number;
  talusAngleDegrees?: number;
  quality?: ErosionQuality;
}

/**
 * DEM loading options.
 */
export interface DemLoadOptions {
  verticalScale?: number;
  generateNormals?: boolean;
  generateMaterialMasks?: boolean;
}

/**
 * Fetch options for URL-based DEM loading.
 */
export interface DemFetchOptions extends DemLoadOptions {
  headers?: HeadersInit;
  signal?: AbortSignal;
}

/**
 * Raw heightmap sample format.
 */
export type RawSampleFormat = "uint16" | "int16" | "float32";

/**
 * Raw heightmap byte order.
 */
export type ByteOrder = "little-endian" | "big-endian";

/**
 * Raw heightmap loading options.
 */
export interface RawHeightmapOptions {
  width: number;
  height: number;
  sampleFormat: RawSampleFormat;
  byteOrder?: ByteOrder;
  metresPerSample: number;
  heightScaleMetres: number;
  noDataValue?: number;
  seaLevelMetres?: number;
}

/**
 * Camera controls in terrain metre space.
 */
export interface CameraOptions {
  position: [number, number, number];
  target: [number, number, number];
  rollDegrees?: number;
  fieldOfViewDegrees: number;
  nearMetres?: number;
  farMetres?: number;
  minimumHeightAboveTerrainMetres?: number;
  allowUnderground?: boolean;
}

/**
 * Sun controls.
 */
export interface SunOptions {
  azimuthDegrees: number;
  elevationDegrees: number;
  intensity: number;
}

/**
 * Atmosphere and haze controls.
 */
export interface AtmosphereOptions {
  rayleighStrength: number;
  mieStrength: number;
  hazeDistanceMetres: number;
  exposure: number;
  skyTint: [number, number, number];
}

/**
 * Water controls.
 */
export interface WaterOptions {
  enabled: boolean;
  seaLevelMetres: number;
  waveScale: number;
  reflectivity: number;
  shorelineSoftnessMetres: number;
}

/**
 * Flora controls.
 */
export interface FloraOptions {
  enabled: boolean;
  density: number;
  treeLineMetres: number;
  seedOffset: number | bigint;
  maxInstances: number;
  /**
   * Tree rendering fidelity. Defaults to `"billboard"`, today's rendering.
   * `"mesh"` is accepted but currently renders as `"cross-quad"` — see
   * `docs/environment-upgrade-plan.md` ("Trees (Mesh tier)").
   */
  treeQuality?: TreeQuality;
  /** Canopy silhouette/colour variety strength, 0 to 1. Defaults to `0`. */
  speciesVariation?: number;
  /** Canopy wind sway strength, 0 to 1. Defaults to `0` (static). */
  windStrength?: number;
}

/**
 * Tree rendering fidelity.
 */
export type TreeQuality = "billboard" | "cross-quad" | "mesh";

/**
 * Grass rendering fidelity.
 */
export type GrassStyle = "billboard-blades" | "dense-blades";

/**
 * Grass ground-cover controls. Grass is a brand-new visual element with no
 * prior equivalent, so it defaults fully `enabled: false`.
 */
export interface GrassOptions {
  enabled: boolean;
  style: GrassStyle;
  density: number;
  viewDistanceMetres: number;
  seedOffset: number | bigint;
  maxInstances: number;
}

/**
 * Cloud rendering fidelity.
 */
export type CloudStyle = "off" | "painted" | "volumetric";

/**
 * Cloud layer controls.
 */
export interface CloudsOptions {
  style: CloudStyle;
  coverage: number;
  speed: number;
  heightMetres: number;
  colour: [number, number, number];
  seedOffset: number | bigint;
  /**
   * Raymarch step count for the `"volumetric"` style only. Always clamped
   * server-side to `8..=64` regardless of the requested value — this bounds
   * a real shader loop, see `docs/environment-upgrade-plan.md` §1.6.
   */
  raymarchSteps?: number;
}

/**
 * Mist/ground-fog rendering fidelity.
 *
 * Distinct from `AtmosphereOptions.hazeDistanceMetres`, which is a uniform,
 * distance-only blend to sky colour. Mist is a height-based ground fog that
 * pools in valleys and near water.
 */
export type MistStyle = "off" | "flat" | "volumetric";

/**
 * Mist/ground-fog controls.
 */
export interface MistOptions {
  style: MistStyle;
  density: number;
  baseHeightMetres: number;
  heightFalloffMetres: number;
  colour: [number, number, number];
  /** Adds extra mist near `WaterOptions.seaLevelMetres`. */
  riseAboveWater: boolean;
  seedOffset: number | bigint;
}

/**
 * Render quality preset.
 */
export type RenderQualityPreset = "preview" | "balanced" | "high" | "offline";

/**
 * Render quality controls.
 */
export interface RenderQualityOptions {
  preset: RenderQualityPreset;
  maxClipmapLevels?: number;
  floraDensityScale?: number;
}

/**
 * Debug overlay names.
 */
export type DebugView =
  | "none"
  | "height"
  | "slope"
  | "normals"
  | "lod"
  | "flow"
  | "materials"
  | "no-data";

/**
 * Render statistics.
 */
export interface RenderStats {
  frameIndex: number;
  frameTimeMs: number;
  gpuFrameTimeMs?: number | null;
  terrainTriangles: number;
  floraInstances: number;
  grassInstances: number;
  clipmapLevels: number;
  activeGpuMemoryBytes?: number | null;
}

/**
 * Heightmap export options.
 */
export interface ExportHeightmapOptions {
  format?: "float32-le";
}

/**
 * Canvas snapshot options.
 */
export interface SnapshotOptions {
  mimeType?: string;
  quality?: number;
}

/**
 * VistaWASM event payloads.
 */
export interface VistaEventMap {
  ready: undefined;
  progress: { phase: string; progress: number };
  warning: { message: string; details?: unknown };
  terrainLoaded: TerrainHandle;
  stats: RenderStats;
  fatalError: Error;
  deviceLost: Error;
}

/**
 * VistaWASM event names.
 */
export type VistaEventName = keyof VistaEventMap;

/**
 * Listener for a VistaWASM event.
 */
export type VistaEventListener<EventName extends VistaEventName> = (
  payload: VistaEventMap[EventName]
) => void;

/**
 * Public VistaWASM engine.
 */
export interface VistaEngine {
  generateFractal(options: FractalTerrainOptions): Promise<TerrainHandle>;
  loadDemFromUrl(url: string, options?: DemFetchOptions): Promise<TerrainHandle>;
  loadDemFromArrayBuffer(buffer: ArrayBuffer, options?: DemLoadOptions): Promise<TerrainHandle>;
  loadRawHeightmap(buffer: ArrayBuffer, options: RawHeightmapOptions): Promise<TerrainHandle>;
  setCamera(camera: CameraOptions): void;
  setSun(sun: SunOptions): void;
  setAtmosphere(atmosphere: AtmosphereOptions): void;
  setWater(water: WaterOptions): void;
  setFlora(flora: FloraOptions): void;
  setGrass(grass: GrassOptions): void;
  setClouds(clouds: CloudsOptions): void;
  setMist(mist: MistOptions): void;
  setRenderQuality(quality: RenderQualityOptions): void;
  setDebugView(debugView: DebugView): void;
  renderOnce(): RenderStats;
  start(): void;
  stop(): void;
  resize(width: number, height: number, devicePixelRatio?: number): void;
  dispose(): void;
  exportHeightmap(options?: ExportHeightmapOptions): Uint8Array<ArrayBuffer>;
  exportSnapshot(options?: SnapshotOptions): Promise<Blob>;
  on<EventName extends VistaEventName>(
    eventName: EventName,
    listener: VistaEventListener<EventName>
  ): () => void;
}
