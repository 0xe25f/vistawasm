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
  setBiomes(biomes: unknown): void;
  biomeAt(x: number, z: number): unknown;
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
  biomes?: BiomeOptions;
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
 * Gerstner wave simulation for open water.
 */
export interface WaveOptions {
  /** Whether swell is simulated. Defaults to `true`. */
  enabled?: boolean;
  /** Trough-to-crest height of the dominant swell, 0 to 30 metres. Defaults to `0.9`. */
  amplitudeMetres?: number;
  /** Wavelength of the dominant swell, up to 2000 metres. Defaults to `38`. */
  wavelengthMetres?: number;
  /** Direction the swell travels towards (0 = +Z, 90 = +X). Defaults to `35`. */
  directionDegrees?: number;
  /** Crest sharpness, 0 (rolling) to 1 (choppy). Defaults to `0.55`. */
  steepness?: number;
  /** Animation speed multiplier. Defaults to `1`. */
  speed?: number;
  /** Spread of secondary waves, 0 (parallel) to 1 (confused sea). Defaults to `0.55`. */
  directionalSpread?: number;
}

/**
 * Rivers and lakes extracted from the terrain drainage network.
 */
export interface RiverOptions {
  /** Whether rivers and lakes are generated. Defaults to `true`. */
  enabled?: boolean;
  /** Upstream catchment area, in km², before a channel becomes a river. Defaults to `1.2`. */
  minCatchmentKm2?: number;
  /** Multiplier on the automatic river width. Defaults to `1`. */
  widthScale?: number;
  /** River current speed multiplier. Defaults to `1`. */
  currentSpeed?: number;
}

/**
 * Water controls.
 */
export interface WaterOptions {
  enabled: boolean;
  seaLevelMetres: number;
  /** Small-scale ripple strength. Large swell is controlled by `waves`. */
  waveScale: number;
  reflectivity: number;
  shorelineSoftnessMetres: number;
  /** Gerstner swell simulation. */
  waves?: WaveOptions;
  /** Rivers and lakes. Changing these re-carves the terrain. */
  rivers?: RiverOptions;
  /** Direction of the open-water surface current. Defaults to `60`. */
  currentDirectionDegrees?: number;
  /** Surface current speed in metres per second. Defaults to `0.35`. */
  currentSpeed?: number;
  /** Colour of shallow water. Defaults to `[0.1, 0.52, 0.5]`. */
  shallowColour?: [number, number, number];
  /** Colour of deep water. Defaults to `[0.015, 0.09, 0.16]`. */
  deepColour?: [number, number, number];
  /** Depth at which the sea bed stops being visible. Defaults to `6`. */
  clarityMetres?: number;
  /** Foam strength, 0 to 1. Defaults to `0.7`. */
  foam?: number;
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
   * Tree rendering fidelity. Defaults to `"mesh"`: full 3D species meshes
   * near the camera, impostors of the same meshes in the distance.
   * `"cross-quad"` and `"billboard"` draw only impostors (cheaper).
   */
  treeQuality?: TreeQuality;
  /** Per-tree size and colour variety, 0 to 1. Defaults to `0.6`. */
  speciesVariation?: number;
  /** Wind sway strength, 0 to 1. Defaults to `0.3`. */
  windStrength?: number;
  /** Distance at which meshes cross-fade to impostors. Defaults to `420`. */
  meshDistanceMetres?: number;
}

/**
 * Tree rendering fidelity.
 */
export type TreeQuality = "billboard" | "cross-quad" | "mesh";

/**
 * Biome names reported by `biomeAt()`.
 */
export type BiomeKind =
  | "grassyMeadows"
  | "outerThicket"
  | "outerForest"
  | "innerForest"
  | "mountainFoothills"
  | "mountainProper"
  | "outerVolcanic"
  | "calderaVolcanic"
  | "savannahExpanse"
  | "coastalBeach"
  | "coastalRocky"
  | "outerJungle"
  | "innerJungle"
  | "swampWetlands"
  | "ocean";

/**
 * Climate-driven biome controls. Every field is optional.
 */
export interface BiomeOptions {
  /** Use climate-driven biomes. When `false`, only height and slope are used. Defaults to `true`. */
  enabled?: boolean;
  /** Seed offset for the climate fields. */
  seedOffset?: number | bigint;
  /** -1 (colder) to 1 (hotter). Defaults to `0`. */
  temperatureBias?: number;
  /** -1 (drier) to 1 (wetter). Defaults to `0`. */
  moistureBias?: number;
  /** Typical climate region size in metres. Defaults to `7000`. */
  climateScaleMetres?: number;
  /** Volcanic regions around high peaks, 0 to 1. Defaults to `0.35`. */
  volcanism?: number;
  /** Height above sea level below which flat ground is beach. Defaults to `5`. */
  beachHeightMetres?: number;
  /** Snow line in metres. Defaults to 80% of the way from sea level to the highest peak. */
  snowLineMetres?: number;
}

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
  /** Wind speed multiplier (1 is roughly 15 m/s). */
  speed: number;
  /** Altitude of the cloud base in metres. */
  heightMetres: number;
  colour: [number, number, number];
  seedOffset: number | bigint;
  /**
   * Raymarch step count for the `"volumetric"` style only. Must be within
   * `8..=64` — this bounds a real shader loop, see
   * `docs/environment-upgrade-plan.md` §1.6.
   */
  raymarchSteps?: number;
  /** Direction the wind carries clouds towards. Defaults to `70`. */
  windDirectionDegrees?: number;
  /** How quickly cloud shapes billow and change, 0 to 1. Defaults to `0.35`. */
  evolution?: number;
  /** Vertical thickness of the cloud layer in metres. Defaults to `1600`. */
  thicknessMetres?: number;
  /** Optical density, 0 (wispy) to 1 (dense cumulus). Defaults to `0.6`. */
  density?: number;
  /** Whether clouds cast moving shadows. Defaults to `true`. */
  castShadows?: boolean;
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
  /** Direction the wind pushes fog banks towards. Defaults to `70`. */
  windDirectionDegrees?: number;
  /** Fog bank drift speed in metres per second. Defaults to `2.5`. */
  windSpeedMetresPerSecond?: number;
  /** Glow when looking towards the sun, 0 to 1. Defaults to `0.6`. */
  sunScattering?: number;
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
  | "no-data"
  | "biomes";

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
  setBiomes(biomes: BiomeOptions): void;
  biomeAt(x: number, z: number): BiomeKind | undefined;
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
