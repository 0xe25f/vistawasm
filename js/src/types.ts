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
  /** The GPU reported a validation or out-of-memory error. */
  | "GPU_ERROR"
  | "ENGINE_DISPOSED"
  /** A malformed image, PNG or bundle; the message says what is wrong. */
  | "INVALID_DEM"
  /** The WASM module or its JavaScript glue could not be fetched or compiled. */
  | "WASM_LOAD_FAILED"
  | "INTERNAL_ERROR";

/**
 * Module override for tests or custom WASM hosting.
 */
export interface VistaWasmInitOptions {
  /**
   * Explicit WASM URL or compiled module passed to the generated loader.
   * By default it is the `.wasm` file next to the package's own glue, as
   * a URL bundlers see and emit; with `moduleUrl` or `wasmModule`, the
   * file next to that glue.
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
  /** Check engine options as `create` would, without applying them. */
  checkOptions(options: unknown): void;
  /**
   * Why the device was lost (once; an empty line while it is fine), then
   * the GPU errors no error scope caught since the last call, one a line,
   * or `undefined` when there is nothing new.
   */
  takeGpuEvents?(): string | undefined;
  generateFractal(
    options: unknown,
    onProgress?: (phase: string, progress: number) => void
  ): Promise<unknown>;
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
  temperatureAt(x: number, z: number): number | undefined;
  setWaterMask(width: number, height: number, data?: Uint8Array): string | undefined;
  setBiomeMap(width: number, height: number, data: Uint8Array | undefined, border: number): string;
  setVegetationMask(grass: boolean, width: number, height: number, data?: Uint8Array): string | undefined;
  getWaterSounds(x: number, y: number, z: number): Float32Array;
  getWaterfalls(): Float32Array;
  getInflows(): Float32Array;
  setDebugView(debugView: unknown): void;
  renderOnce(): unknown;
  /** The frame index, negated when the weather changed; see `renderFrame` in `api.rs`. */
  renderFrame?(): number;
  resize(width: number, height: number, devicePixelRatio?: number): void;
  dispose(): void;
  exportHeightmap(): Uint8Array<ArrayBuffer>;
  exportMap(kind: number, width?: number, height?: number): RawExportedMap;
  exportTrees(region?: Float32Array, maxCount?: number): Float32Array;
  getStats(): unknown;
  setWeather(weather: unknown): void;
  getWeather(): unknown;
  getWeatherPresets(): unknown;
  weatherAt(x: number, z: number): Float32Array;
  advanceWeather(seconds: number): void;
  setTimeOfDay(
    enabled: boolean,
    hours: number,
    dayLengthMinutes: number,
    latitudeDegrees: number,
    dayOfYear: number
  ): void;
  getTimeOfDay(): Float32Array;
  setShadows(shadows: unknown): void;
  setSurface(surface: unknown): void;
  setTreeModel(
    species: unknown,
    positions: Float32Array,
    normals: Float32Array,
    uvs: Float32Array,
    indices: Uint32Array,
    textureLayers?: Float32Array,
    wind?: Float32Array
  ): void;
  resetTreeModel(species: unknown): void;
  setTreeInstances(packed?: Float32Array): void;
  replaceTexture(target: unknown, layer: number, rgba: Uint8Array): void;
  resetTextures(): void;
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
  weather?: WeatherOptions;
  shadows?: ShadowOptions;
  surface?: SurfaceOptions;
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
  /**
   * Samples a side: a power of two from 16 to 2048 (`MAX_TERRAIN_SIDE`),
   * set by the 4 GiB of memory a WASM module can address.
   */
  size: 512 | 1024 | 2048 | number;
  horizontalScaleMetres: number;
  verticalScale: number;
  baseHeightMetres?: number;
  seaLevelMetres?: number;
  noise: NoiseOptions;
  shape?: TerrainShapeOptions;
  /** Erosion. Unset fields take the landform's defaults. */
  erosion?: ErosionOptions;
  /** Character of the land. Defaults to "continental". */
  landform?: LandformKind;
  /**
   * "coast" (default) keeps the land inside the map, ringed by sea.
   * "open" lets land run to the map edge, for tiling several maps.
   */
  edges?: TerrainEdges;
}

/**
 * What happens where a generated map meets its square edge.
 *
 * - `coast`: the land falls away to sea along a natural coastline.
 * - `open`: the land runs to the edge, as it would on a tile of a larger map.
 */
export type TerrainEdges = "coast" | "open";

/**
 * The character of a generated map.
 *
 * - `continental`: mixed plains, hills and one or two eroded ranges.
 * - `alpine`: high, heavily eroded ranges with deep glacial valleys.
 * - `rollingHills`: gentle downs and broad vales, no ranges.
 * - `archipelago`: many islands of varied size.
 * - `mesaDesert`: terraced plateaus, buttes and canyons.
 * - `fjords`: steep coastal ranges cut by flooded U-shaped valleys.
 * - `volcanicIsland`: a central cone with a caldera and a reef shelf.
 */
export type LandformKind =
  | "continental"
  | "alpine"
  | "rollingHills"
  | "archipelago"
  | "mesaDesert"
  | "fjords"
  | "volcanicIsland";

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
  /**
   * Most bytes to read, at most and by default `MAX_DEM_BYTES`. A
   * response whose `Content-Length` is larger is refused before it is
   * read, and one that grows past it is cancelled.
   */
  maxBytes?: number;
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
  /** Samples per row, 2 to 2048 (WebAssembly's limit on terrain size). */
  width: number;
  /** Rows, 2 to 2048. */
  height: number;
  sampleFormat: RawSampleFormat;
  byteOrder?: ByteOrder;
  metresPerSample: number;
  heightScaleMetres: number;
  noDataValue?: number;
  seaLevelMetres?: number;
  /**
   * The landform the heights were generated as, which sets the bedrock's
   * beds; `loadBundle()` passes the exported one. Defaults to
   * `"continental"`.
   */
  landform?: LandformKind;
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
  /** Upstream catchment area, in km², before a channel becomes a river. Defaults to `0.15`. */
  minCatchmentKm2?: number;
  /** Multiplier on the automatic river width. Defaults to `1`. */
  widthScale?: number;
  /** River current speed multiplier. Defaults to `1`. */
  currentSpeed?: number;
  /**
   * How strongly snow fields and glaciers feed streams, from 0 (no
   * snowmelt) to 2. Streams also start at glacier snouts and at the lower
   * edge of snowy peaks. Defaults to `1`.
   */
  snowmelt?: number;
  /** Small springs at the foot of steep slopes. Defaults to `true`. */
  springs?: boolean;
  /**
   * How strongly lowland bends migrate by bank erosion, from 0 (rivers
   * follow their valley without meandering) to 1. Defaults to `0.6`.
   */
  meanders?: number;
  /**
   * How long meanders have been developing, from 0 (young, gentle bends)
   * to 1 (mature loops, neck cut-offs and oxbow lakes). Defaults to `0.5`.
   */
  meanderMaturity?: number;
  /**
   * Braided threads between gravel bars on steep, wide, unconfined valley
   * floors, from 0 (never) to 1 (wherever slope and discharge call for
   * them). Defaults to `1`.
   */
  braiding?: number;
  /** Waterfalls where rivers cross cliffs. Defaults to `true`. */
  waterfalls?: boolean;
  /**
   * Water arriving from beyond the map. `"auto"` (default) adds one inflow
   * where a valley meets an open map edge, sized from a basin ten times
   * the map's land area; `"none"` adds nothing; or up to 8 explicit inflows.
   */
  inflow?: "auto" | "none" | RiverInflow[];
  /**
   * Bankside greening and plants, 0 (off) to 2. Default 1. Ground within
   * 25 to 400 m of rivers (wider by discharge) and 40 m of lakes grows
   * moister, greener and more wooded, and a band of tufts, tall herbs and
   * scrub lines every channel (0 removes it, 2 doubles it).
   */
  riparian?: number;
}

/**
 * Water entering the map from beyond it, for `RiverOptions.inflow`.
 */
export interface RiverInflow {
  /** World x and z in metres. Snaps to the nearest land sample. */
  position: [number, number];
  /** Mean discharge, 0 to 100,000 m³/s. */
  dischargeCubicMetresPerSecond: number;
}

/**
 * An inflow in use, from `getInflows()`.
 */
export interface WaterInflow {
  /** Where the water enters, on the land sample it snapped to, in world metres. */
  position: [number, number, number];
  dischargeCubicMetresPerSecond: number;
}

/**
 * Painted water for `setWaterMask()`.
 */
export interface WaterMask {
  /** Samples per row, 2 to 2048 (the largest terrain it is resampled to). */
  width: number;
  /** Rows, 2 to 2048. */
  height: number;
  /**
   * One byte per sample, row-major, north row first:
   * 0 = no water, 1 to 127 = river brush strength, 128 to 255 = lake/pond.
   * A strength of 1 paints a river 1 m wide, and 127 one 60 m wide.
   */
  data: Uint8Array;
}

/**
 * A painted biome map for `setBiomeMap()`.
 */
export interface BiomeMap {
  /** Samples per row, 2 to 2048 (the largest terrain it is resampled to). */
  width: number;
  /** Rows, 2 to 2048. */
  height: number;
  /** One BiomeKind index per sample, row-major, north row first; 255 = not painted. */
  data: Uint8Array;
  /** Width of the dithered border in samples, 0 to 8. Defaults to 3. */
  borderSamples?: number;
}

/**
 * A tree or grass density mask for `setVegetationMasks()`.
 */
export interface DensityMask {
  /** Samples per row, 2 to 2048 (the largest terrain it is resampled to). */
  width: number;
  /** Rows, 2 to 2048. */
  height: number;
  /**
   * One byte per sample, row-major, north row first: 0 = none, 128 =
   * unchanged, 255 = twice as dense (at most as dense as density 4),
   * linear between.
   */
  data: Uint8Array;
}

/**
 * `setVegetationMasks()` input. An omitted key keeps its current mask;
 * `null` clears it.
 */
export interface VegetationMasks {
  trees?: DensityMask | null;
  grass?: DensityMask | null;
}

/**
 * The painted inputs in effect, as `getPaintedMaps()` returns them and
 * `exportBundle()` writes them.
 */
export interface PaintedMaps {
  biome?: BiomeMap;
  water?: WaterMask;
  trees?: DensityMask;
  grass?: DensityMask;
}

/**
 * An image to import: a `Blob` (such as a `File`), or its bytes. PNGs
 * are decoded at full precision; other formats the browser decodes, at 8
 * bits.
 */
export type ImageSource = Blob | ArrayBuffer | Uint8Array;

/** Which channel of an image to read. */
export type ImageChannel = "luminance" | "r" | "g" | "b" | "a";

/**
 * `loadHeightmapImage()` options.
 */
export interface HeightmapImageImportOptions {
  /** Metres between samples, over 0. */
  metresPerSample: number;
  /**
   * Height of black. Defaults to the PNG's `vistawasm:range` text, which
   * 16-bit exports carry; otherwise required.
   */
  minHeightMetres?: number;
  /** Height of white. Defaults as `minHeightMetres` does. */
  maxHeightMetres?: number;
  /** Sea level in metres. Defaults to 0. */
  seaLevelMetres?: number;
  /** The channel read as height. Defaults to `"luminance"`. */
  channel?: ImageChannel;
}

/**
 * One custom colour for `biomeMapFromImage()`.
 */
export interface BiomeLegendEntry {
  /** sRGB colour, each component 0 to 255. */
  colour: [number, number, number];
  biome: BiomeKind;
}

/**
 * `biomeMapFromImage()` options.
 */
export interface BiomeMapFromImageOptions {
  /** Your own colours, used instead of the exported biome map's legend. */
  legend?: BiomeLegendEntry[];
  /** Width of the dithered border in samples, 0 to 8. Defaults to 3. */
  borderSamples?: number;
  /** Called with each warning. Defaults to `console.warn`. */
  onWarning?: (message: string) => void;
}

/**
 * `waterMaskFromImage()` options.
 */
export interface WaterMaskFromImageOptions {
  /**
   * `"legend"` (the default) reads the exported water map's colours:
   * lake blues are lakes, river blues and cyans are rivers whose
   * brightness sets their strength. `"grey"` reads values as they are,
   * with the `WaterMask` meanings.
   */
  mode?: "legend" | "grey";
  /** Called with each warning. Defaults to `console.warn`. */
  onWarning?: (message: string) => void;
}

/**
 * `densityMaskFromImage()` options.
 */
export interface DensityMaskFromImageOptions {
  /** The channel read. Defaults to `"luminance"`. */
  channel?: ImageChannel;
  /** Called with each warning. Defaults to `console.warn`. */
  onWarning?: (message: string) => void;
}

/**
 * `loadBundle()` options.
 */
export interface BundleLoadOptions {
  /** Apply the bundle's options through the engine's setters. Defaults to `true`. */
  applySettings?: boolean;
}

/**
 * The images `loadTerrainFromImages()` loads together. Only `height` is
 * required.
 */
export interface TerrainImages {
  height: ImageSource;
  biome?: ImageSource;
  water?: ImageSource;
  trees?: ImageSource;
  grass?: ImageSource;
}

/**
 * The loudest water sound of one kind near a listener.
 */
export interface WaterSound {
  /** Distance from the listener to the source, in metres. */
  distanceMetres: number;
  /** Loudness from 0 to 1: the source's strength over its distance squared. */
  loudness: number;
  /** Where the sound comes from, in world metres. */
  position: [number, number, number];
}

/**
 * Water sounds near a listener. Each is null when there is no such
 * source within its search radius: 400 m for rivers and lake shores,
 * 1500 m for waterfalls and 600 m for surf. Frozen water is silent.
 */
export interface WaterSounds {
  river: WaterSound | null;
  waterfall: WaterSound | null;
  lakeShore: WaterSound | null;
  surf: WaterSound | null;
}

/**
 * A waterfall.
 */
export interface Waterfall {
  /** Where the water lands, at the river level at the foot of the fall, in world metres. */
  position: [number, number, number];
  heightMetres: number;
  widthMetres: number;
  dischargeCubicMetresPerSecond: number;
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
  /**
   * What water reflects. `"screen"` (default) reflects the terrain, trees
   * and banks on screen, falling back to the sky and clouds where a
   * reflected ray leaves the screen; `"sky"` reflects the sky and clouds
   * only, which costs less.
   */
  reflections?: "screen" | "sky";
  /**
   * Eddies and vortices in flowing rivers, from 0 (off) to 1. Defaults to
   * `1`. Changing it does not rebuild rivers.
   */
  eddies?: number;
  /**
   * Refraction of the bed and caustics in shallow water, from 0 (off) to
   * 1. Defaults to `1`. Changing it does not rebuild rivers.
   */
  refraction?: number;
}

/**
 * Flora controls.
 */
export interface FloraOptions {
  enabled: boolean;
  /**
   * Tree density, 0 to 4. 1 is the old maximum; 4 is a closed canopy
   * where the land supports it. Defaults to 0.35.
   */
  density: number;
  treeLineMetres: number;
  seedOffset: number | bigint;
  /**
   * Most trees generated at once: the static far set plus the tiles
   * streamed around the camera, 0 to 4,000,000.
   */
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
  /**
   * Distance at which meshes cross-fade to impostors, which the nine-view
   * impostors take over from 150 m however far this reaches. Full meshes
   * give way to lighter ones at 50 m (or a third of this). Defaults to
   * `420`.
   */
  meshDistanceMetres?: number;
  /**
   * Per-biome rules that replace the built-in species mix. Biomes without
   * a rule keep the built-in mix. Defaults to `[]`.
   */
  speciesRules?: FloraRule[];
  /**
   * Distinct grown shapes per species, 1 to 4. Defaults to 4. Lower uses
   * less GPU memory: each variant has four age classes at two levels of
   * detail, and its own impostors. More variants grow over the frames
   * after the first.
   */
  variantsPerSpecies?: number;
}

/**
 * Tree species slots. Each slot has a procedural model that
 * `setTreeModel()` can replace.
 */
export type TreeSpecies =
  | "oak"
  | "pine"
  | "spruce"
  | "palm"
  | "jungle"
  | "cypress"
  | "acacia"
  | "shrub";

/**
 * Replaces the species mix for one biome.
 */
export interface FloraRule {
  biome: BiomeKind;
  /** Species by relative weight. An empty list keeps the biome treeless. */
  species: { species: TreeSpecies; weight: number }[];
  /** Multiplier on the biome's tree density, 0 to 4. Defaults to `1`. */
  density?: number;
}

/**
 * A host-supplied tree model. Units are metres with the base of the trunk
 * at the origin and +Y up.
 */
export interface TreeModel {
  /** Three numbers per vertex. At most 65,536 vertices. */
  positions: Float32Array | number[];
  /** Three numbers per vertex. */
  normals: Float32Array | number[];
  /** Two numbers per vertex. */
  uvs: Float32Array | number[];
  /** Three indices per triangle. */
  indices: Uint32Array | number[];
  /**
   * Flora texture layer per vertex (0 to 14, see `docs/hooks.md`). Layers
   * 4 and above are alpha-tested foliage. Defaults to `0` (oak bark).
   */
  textureLayers?: Float32Array | number[];
  /** Sway weight per vertex, 0 (rigid) to 1. Defaults to rising with height. */
  wind?: Float32Array | number[];
}

/**
 * One host-placed tree.
 */
export interface TreePlacement {
  x: number;
  /** Height of the base of the trunk in metres. */
  y: number;
  z: number;
  species: TreeSpecies;
  /** Uniform scale, above 0 and at most 20. Defaults to `1`. */
  scale?: number;
  /** Rotation around the vertical axis in radians. Defaults to `0`. */
  rotation?: number;
  /** Colour variation, 0 to 1. Defaults to `0.5`. */
  tint?: number;
  /** Colour dryness, 0 (lush) to 1 (dry). Defaults to `0`. */
  dryness?: number;
  /**
   * Snap this tree to the rendered ground, sinking its roots on slopes.
   * The engine then ignores `y`. Defaults to `false`.
   */
  ground?: boolean;
}

/**
 * A texture array that host textures can replace, one layer at a time.
 */
export type TextureTarget = "terrainAlbedo" | "terrainNormal" | "flora";

/**
 * The built-in weather presets.
 */
export type BuiltInWeatherKind =
  | "clear"
  | "fewClouds"
  | "partlyCloudy"
  | "brokenClouds"
  | "overcast"
  | "mist"
  | "fog"
  | "lightRain"
  | "rain"
  | "heavyRain"
  | "storm"
  | "snow"
  | "blizzard";

/**
 * A weather preset's name: a built-in preset, or a custom one added
 * through `WeatherOptions.presets`. It is always a string.
 */
export type WeatherKind = BuiltInWeatherKind | (string & {});

/**
 * One kind of weather. Every field is optional: a custom preset takes
 * unset fields from the preset it `extends` (or from `"partlyCloudy"`),
 * and an override of a built-in preset changes only the fields given.
 * Unknown fields are rejected.
 */
export interface WeatherPreset {
  /** The preset to inherit unset fields from. */
  extends?: WeatherKind;
  /** Mean cloud coverage, 0 to 1. */
  cloudCoverage?: number;
  /** Cloud density, 0 to 1. */
  cloudDensity?: number;
  /** Multiplier on the cloud layer's thickness, 0.2 to 3. */
  cloudThickness?: number;
  /** Cloud type, 0 (cumulus) to 1 (flat sheet). */
  stratiform?: number;
  /** Towering storm clouds, 0 to 1. */
  towering?: number;
  /** Darkening of cloud bases, 0 to 1. */
  baseDarkness?: number;
  /** Raggedness of cloud bases, 0 to 1. */
  raggedBase?: number;
  /** Curtains of rain or snow below the clouds, 0 to 1. */
  rainShafts?: number;
  /** High cirrus, 0 to 1. */
  cirrus?: number;
  /** Relative humidity, 0 to 1: damp air whitens and thickens the haze. */
  humidity?: number;
  /** Aerosol turbidity, 2 (clean alpine air) to 10 (thick tropical haze). */
  turbidity?: number;
  /** Multiplier on the haze distance, 0.05 to 2 (lower is hazier). */
  hazeDistanceScale?: number;
  /** Ground mist density, 0 to 1. */
  mistDensity?: number;
  /** Added to the climate's temperature, -30 to 30 °C. */
  temperatureOffsetCelsius?: number;
  /** Mean wind speed, 0 to 60 m/s. */
  windMetresPerSecond?: number;
  /** Gust strength relative to the mean wind, 0 to 1. */
  gustiness?: number;
  /** Rain intensity, 0 to 2 (above 1, a downpour). */
  rain?: number;
  /** Snowfall intensity, 0 to 2. */
  snow?: number;
  /** Lightning flashes per minute, 0 to 60. */
  lightningPerMinute?: number;
  /** How far coverage varies across the regional map, 0 to 1. */
  coverageSpread?: number;
  /** How far precipitation varies under thick cloud, 0 to 1. */
  precipitationSpread?: number;
  /** Size of weather cells, 1 to 100 km. */
  cellSizeKm?: number;
  /** 0 for smooth fields, 1 for discrete storm cells. */
  cellularity?: number;
  /** Variation of low-cloud base height, 0 to 0.2 of the layer thickness. */
  baseVariation?: number;
  /** Cotton-wool lumps under low clouds, 0 to 1. */
  baseLumpiness?: number;
  /** Rippled mid-level cloudlets (altocumulus), 0 to 1. */
  altocumulus?: number;
  /** Grey mid-level veil that dims the sun (altostratus), 0 to 1. */
  altostratus?: number;
  /** Height of the mid-level layer, 2000 to 7000 metres. */
  altoHeightMetres?: number;
  /** Mid-level drift speed in units of 15 m/s, 0 to 4. */
  altoSpeed?: number;
  /** Weights of the presets that may follow this one when cycling. */
  next?: Record<string, number>;
  /** Shortest time the preset lasts when cycling, 1 to 86400 seconds. */
  minDurationSeconds?: number;
  /** Longest time the preset lasts when cycling, 1 to 86400 seconds. */
  maxDurationSeconds?: number;
  /** Temperatures at the camera the preset suits, -60 to 60 °C. */
  climate?: { minCelsius?: number; maxCelsius?: number };
}

/**
 * A fully resolved preset, as `getWeatherPresets()` reports it.
 */
export type ResolvedWeatherPreset = Required<Omit<WeatherPreset, "extends">>;

/**
 * The weather at one place, as `weatherAt()` reports it. Every value is 0
 * to 1, but `precipitation`, which is above 1 in a downpour.
 */
export interface LocalWeather {
  coverage: number;
  /** Rain and snow falling, together. */
  precipitation: number;
  /** How much of a storm cell is overhead. */
  storminess: number;
  humidity: number;
  /** Wetness of the ground. */
  wetness: number;
  /** Puddle water on the ground. */
  puddles: number;
  /** Depth of settled snow (at least the snow that lies all year). */
  snowDepth: number;
}

/**
 * Time of day controls. Every field is optional.
 */
export interface TimeOfDayOptions {
  /**
   * Whether the sun follows the time of day. While it does, `setSun`
   * sets only the sun's intensity. Defaults to `false`.
   */
  enabled?: boolean;
  /** Local time, 0 to 24. Defaults to `12`. */
  hours?: number;
  /** Real minutes for a full day, 1 to 1440. Defaults to `24`. */
  dayLengthMinutes?: number;
  /** Latitude, -89 to 89 degrees (north positive). Defaults to `45`. */
  latitudeDegrees?: number;
  /** Day of the year, 1 to 366. Defaults to `172`. */
  dayOfYear?: number;
}

/**
 * The time of day, as `getTimeOfDay()` reports it.
 */
export interface TimeOfDay {
  hours: number;
  /** In the same convention as `SunOptions.azimuthDegrees`. */
  sunAzimuthDegrees: number;
  /** Above the horizon, with refraction. */
  sunElevationDegrees: number;
  /** `null` during polar day or night. */
  sunriseHours: number | null;
  /** `null` during polar day or night. */
  sunsetHours: number | null;
}

/**
 * Which systems the weather drives. Anything switched off keeps its own
 * manual settings. Every effect defaults to `true`.
 */
export interface WeatherEffects {
  clouds?: boolean;
  mist?: boolean;
  wind?: boolean;
  water?: boolean;
  precipitation?: boolean;
  ground?: boolean;
  lightning?: boolean;
}

/**
 * Weather pattern controls. Every field is optional.
 */
export interface WeatherOptions {
  /** Whether the weather system is active. Defaults to `false`. */
  enabled?: boolean;
  /**
   * The preset to move towards (or start from, with `autoCycle`): a
   * built-in preset or a custom one from `presets`. Defaults to
   * `"partlyCloudy"`.
   */
  state?: WeatherKind;
  /** Automatically move on to new weather over time. Defaults to `false`. */
  autoCycle?: boolean;
  /** Average time each state lasts when cycling. Defaults to `240`. */
  stateDurationSeconds?: number;
  /** Time taken to blend between states. Defaults to `30`. */
  transitionSeconds?: number;
  /** Whether snow may occur when cycling. Defaults to `false`. */
  allowSnow?: boolean;
  /** Seed for the weather sequence and gusts. Defaults to `4111`. */
  seedOffset?: number | bigint;
  /** Direction the wind blows towards. Defaults to `70`. */
  windDirectionDegrees?: number;
  /** Multiplier on wind speed, 0 to 4. Defaults to `1`. */
  windScale?: number;
  /**
   * Multiplier on rain and snow, 0 to 2. Above 1, rain becomes a downpour:
   * more and longer streaks, and a grey veil that cuts visibility.
   * Defaults to `1`.
   */
  precipitationScale?: number;
  /** Raindrops that land on the lens and run down the screen. Defaults to `false`. */
  lensDrops?: boolean;
  /** Drops on the lens at once in full rain, 0 to 512. Scales with rain intensity. Defaults to 60. */
  lensDropCount?: number;
  /** Smallest lens drop diameter as a fraction of the canvas height, 0.002 to 0.2. Defaults to 0.008. */
  lensDropMinSize?: number;
  /** Largest lens drop diameter as a fraction of the canvas height, 0.002 to 0.2. Defaults to 0.05. */
  lensDropMaxSize?: number;
  effects?: WeatherEffects;
  /** Custom presets and overrides of built-in presets, by name. */
  presets?: Record<string, WeatherPreset>;
  /** Weather varies across the map. Defaults to `true`. */
  regional?: boolean;
  /** Size of the regional weather map in km, 16 to 256. Defaults to `64`. */
  regionSizeKm?: number;
}

/**
 * The current, blended weather reported by `getWeather()`.
 */
export interface WeatherState {
  from: WeatherKind;
  to: WeatherKind;
  /** Blend from `from` (0) to `to` (1). */
  blend: number;
  cloudCoverage: number;
  cloudDensity: number;
  mistDensity: number;
  windSpeedMetresPerSecond: number;
  windDirectionDegrees: number;
  rain: number;
  snow: number;
  /** Ground wetness; lags behind the rain. */
  wetness: number;
  /** Settled snow; builds up and melts slowly. */
  snowCover: number;
  lightning: number;
  /** Cloud type the weather is using, 0 (cumulus) to 1 (flat sheet). */
  stratiform: number;
  towering: number;
  baseDarkness: number;
  raggedBase: number;
  rainShafts: number;
  /** Relative humidity at the camera. */
  humidity?: number;
  /** Aerosol turbidity, 2 to 10. */
  turbidity?: number;
  /** How much of a storm cell is over the camera. */
  storminess?: number;
  /** Share of direct sunlight the clouds let through towards the camera. */
  sunTransmittance?: number;
  /** Gust strength relative to the mean wind. */
  gustiness?: number;
  /** Puddle water under the camera. */
  puddles?: number;
}

/**
 * Shadow controls. Every field is optional.
 */
export interface ShadowOptions {
  terrain?: {
    /** Defaults to `true`. */
    enabled?: boolean;
    /** Darkness, 0 to 1. Defaults to `0.9`. */
    strength?: number;
    /** Penumbra width, 0 to 1. Defaults to `0.35`. */
    softness?: number;
  };
  trees?: {
    /** Defaults to `true`. */
    enabled?: boolean;
    /** Radius around the camera that trees shadow, 10 to 4000. Defaults to `260`. */
    distanceMetres?: number;
    /** Shadow map size. Defaults to `2048`. */
    resolution?: 512 | 1024 | 2048 | 4096;
    /** Darkness, 0 to 1. Defaults to `0.8`. */
    strength?: number;
    /** Filter width, 0 to 1. Defaults to `0.5`. */
    softness?: number;
  };
  clouds?: {
    /** Defaults to `true`. Also requires `CloudsOptions.castShadows`. */
    enabled?: boolean;
    /** Darkness, 0 to 1. Defaults to `0.78`. */
    strength?: number;
  };
}

/**
 * Terrain surface shading controls. Every field is optional.
 */
export interface SurfaceOptions {
  /** Detailed textured materials; `false` uses flat colours. Defaults to `true`. */
  textures?: boolean;
  /** Detail normal maps. Defaults to `true`. */
  detailNormals?: boolean;
  /** Multiplier on texture size, 0.05 to 20. Defaults to `1`. */
  textureScale?: number;
  /**
   * Colour multiplier per material (0 to 4 per channel), in the order
   * lush grass, dry grass, forest floor, sand, rock, snow, mud, volcanic,
   * glacier ice, tundra, river gravel, scree. Lists of 8, 10, 11 or 12
   * are accepted; missing materials stay untinted. Boulders take the rock
   * tint.
   */
  materialTints?: [number, number, number][];
  /**
   * How readily bedrock shows through thin soil, 0 (deep soil everywhere)
   * to 2 (rocky). Rock shows on crests, steep ground and harder beds of
   * the bedrock, and scree lies below it. Defaults to `1`.
   */
  rockiness?: number;
  /** Boulders and talus below outcrops. Defaults to `true`. */
  boulders?: boolean;
  /** Distance in metres within which boulders are drawn, 50 to 1000. Defaults to `300`. */
  boulderDistanceMetres?: number;
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
  | "ocean"
  | "alpineTransition"
  | "lowerSnowyPeaks"
  | "upperSnowyPeaks"
  | "iceArctic";

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
  /**
   * Mean annual temperature at sea level in °C, from -30 to 35. Unset
   * keeps the default climate. Below about -2 °C lowlands freeze into
   * ice sheets; around 15 °C only the highest summits hold ice.
   */
  meanTemperatureCelsius?: number;
}

/**
 * Grass rendering fidelity.
 */
export type GrassStyle = "billboard-blades" | "dense-blades";

/**
 * Grass ground-cover controls. Grass is on by default, at density 0.5: a
 * natural meadow near the camera costs well under a millisecond a frame.
 */
export interface GrassOptions {
  enabled: boolean;
  style: GrassStyle;
  /**
   * Grass density, 0 to 4: how a meadow looks near the camera. 0.5 (the
   * default) is a natural, dense meadow covering at least 70 % of the
   * ground, 1 a lush, taller meadow, and 4 long, dense grass.
   */
  density: number;
  viewDistanceMetres: number;
  seedOffset: number | bigint;
  /** Most tufts generated around the camera at once, 0 to 4,000,000. */
  maxInstances: number;
  /** Ferns and undergrowth replace grass under dense canopy. Defaults to true. */
  forestFloor?: boolean;
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
   * `8..=64`: it bounds a real shader loop, so a larger value could hang
   * the GPU.
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
  /**
   * Cloud render resolution relative to the canvas, 0.25 to 1. Lower is
   * faster; the soft clouds hide the difference. Defaults to `0.5`.
   */
  resolutionScale?: number;
  /**
   * Amount of thin, high cirrus above the main clouds, 0 (none) to 1.
   * Defaults to `0.35`.
   */
  cirrus?: number;
  /** Altitude of the cirrus layer, 1000 to 20000 metres. Defaults to `9000`. */
  cirrusHeightMetres?: number;
  /**
   * Cirrus drift speed in units of 15 m/s, 0 to 10, separate from `speed`.
   * The weather system does not change it. Defaults to `0.4`.
   */
  cirrusSpeed?: number;
  /**
   * Reuse distant clouds between frames: each frame raymarches one sky
   * pixel in every 2 x 2 block and reprojects the other three from the
   * previous frame, cutting the cost of sky clouds to about a quarter.
   * Volumetric only; off automatically while the camera is in or near the
   * cloud layer. Defaults to `false`.
   */
  temporal?: boolean;
  /**
   * Cloud type, 0 (heaped cumulus) to 1 (a flat sheet such as stratus or
   * nimbostratus). Volumetric only. Defaults to `0`.
   */
  stratiform?: number;
  /**
   * Towering storm clouds (cumulonimbus) with anvil tops, 0 to 1. Towers
   * rise well above `thicknessMetres`. Volumetric only. Defaults to `0`.
   */
  towering?: number;
  /** Darker, rain-laden cloud bases, 0 to 1. Defaults to `0`. */
  baseDarkness?: number;
  /** Ragged cloud bases with loose scraps of cloud below, 0 to 1. Defaults to `0`. */
  raggedBase?: number;
  /** Curtains of rain or snow hanging below the clouds, 0 to 1. Defaults to `0`. */
  rainShafts?: number;
  /**
   * Variation of low-cloud base height, from cloud to cloud and within each
   * cloud, 0 (one flat level) to 0.2 of the layer thickness. Volumetric
   * only. Defaults to `0.07`.
   */
  baseVariation?: number;
  /**
   * Cotton-wool lumps under low clouds, 0 (wispy undersides) to 1.
   * Volumetric only. Defaults to `0.6`.
   */
  baseLumpiness?: number;
  /** Rippled mid-level patches (altocumulus, a "mackerel sky"), 0 to 1. Defaults to `0`. */
  altocumulus?: number;
  /** Grey mid-level veil that dims the sun to a watery disc (altostratus), 0 to 1. Defaults to `0`. */
  altostratus?: number;
  /**
   * Height of the mid-level layer in metres, 2000 to 7000. It is kept 300 m
   * above the top of the low clouds and, where there is room, 500 m below
   * the cirrus. Defaults to `4200`.
   */
  altoHeightMetres?: number;
  /**
   * Mid-level drift speed in units of 15 m/s, 0 to 4, along the cloud wind
   * veered 20 degrees. Defaults to `1`.
   */
  altoSpeed?: number;
}

/**
 * Mist/ground-fog rendering fidelity.
 *
 * Distinct from `AtmosphereOptions.hazeDistanceMetres`, which is distance
 * haze that thins gently with altitude. Mist is a height-based ground fog that
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
  /**
   * Sets any distance below that is left unset: `"preview"` 6 km render,
   * 400 m detail, 12 km clouds; `"balanced"` unlimited, 2 km, 60 km;
   * `"high"` unlimited, 5 km, 90 km; `"offline"` unlimited, unlimited,
   * 90 km.
   */
  preset: RenderQualityPreset;
  maxClipmapLevels?: number;
  floraDensityScale?: number;
  /**
   * Render distance in metres, at least 100: terrain, trees, and water
   * beyond it are not drawn, hidden by horizon-coloured fog that is
   * complete at this distance.
   */
  renderDistanceMetres?: number;
  /**
   * Length in metres of the fog band before the render distance, over which
   * the fog thickens from none to complete; 0 or more, capped at the render
   * distance. Defaults to a third of the render distance.
   */
  renderFadeMetres?: number;
  /**
   * Terrain detail distance in metres, at least 100: beyond it, terrain
   * takes one far-scale texture sample per material instead of up to
   * eight.
   */
  detailDistanceMetres?: number;
  /**
   * Cloud render distance in metres, at least 100: clouds and rain
   * curtains are raymarched only this far.
   */
  cloudDistanceMetres?: number;
  /**
   * Length in metres of the band before the cloud distance over which
   * clouds thin out to nothing; 0 or more, capped at the cloud distance.
   * Defaults to 30 % of the cloud distance.
   */
  cloudFadeMetres?: number;
  /**
   * Frame-rate cap for `engine.start()`, in frames per second. Frames are
   * rendered evenly spaced, so a 60 cap on a 144 Hz display gives a steady
   * 60. `0` renders on every animation frame. Defaults to 60.
   */
  maxFrameRate?: number;
  /**
   * Fraction of the canvas resolution the scene is rendered at, from 0.25
   * to 1; a final pass upscales and sharpens it. With dynamic resolution
   * on, this is the highest scale used. Defaults to 1.
   */
  renderScale?: number;
  /**
   * Lower the render scale when frames arrive late, and raise it again when
   * there is time to spare, to hold `maxFrameRate` (or 60 when uncapped).
   * Defaults to `true`.
   */
  dynamicResolution?: boolean;
  /**
   * Lowest render scale dynamic resolution may use, from 0.25 to 1.
   * Defaults to 0.5.
   */
  minRenderScale?: number;
  /**
   * Radius in metres of full-density vegetation around the camera, at
   * least 50. From the preset: 120, 250, 400 or 600.
   */
  vegetationDetailMetres?: number;
  /**
   * Distance where individual trees give way to a canopy layer, at least
   * 50 m. From the preset: 1500, 2500, 4000 or 8000.
   */
  canopyDistanceMetres?: number;
  /**
   * Most tree instances drawn per frame, 1,000 to 4,000,000. From the
   * preset: 40,000, 120,000, 250,000 or 1,000,000.
   */
  maxTreeInstances?: number;
  /**
   * Most grass tufts drawn per frame, 1,000 to 4,000,000. From the
   * preset: 150,000, 400,000, 800,000 or 2,000,000.
   */
  maxGrassInstances?: number;
  /**
   * Most tree triangles drawn per frame, 100,000 to 50,000,000: meshes,
   * impostors and shadow casters. Over it, the furthest full-mesh trees
   * become impostors first, and shadow casters keep to a quarter of it.
   * From the preset: 1,000,000, 2,500,000 or 5,000,000, and no limit for
   * `offline`.
   */
  maxTreeTriangles?: number;
  /**
   * Radius in metres of full-cover grass around the camera, 10 to 300.
   * Beyond it tufts thin out, and the ground's own grass sheen carries
   * the meadow. From the preset: 25, 45, 70 or 120.
   */
  grassDetailMetres?: number;
  /**
   * Time the trees pass as three passes (canopy meshes, understorey
   * meshes and impostors), reported in `gpuPassTimesMs.treeMeshes`,
   * `understorey` and `treeImpostors`. For profiling: the extra passes
   * cost a little. Defaults to `false`.
   */
  splitTreeTiming?: boolean;
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
  /** Trees drawn this frame, estimated on the CPU from the cover. */
  floraInstances: number;
  /** Grass tufts and reeds drawn this frame, estimated likewise. */
  grassInstances: number;
  clipmapLevels: number;
  activeGpuMemoryBytes?: number | null;
  /** The dominant weather, or `null` when the weather system is off. */
  weather?: WeatherKind | null;
  /**
   * GPU time per pass, when the browser supports timestamp queries.
   * Measured a few frames behind, without stalling rendering.
   */
  gpuPassTimesMs?: GpuPassTimes | null;
  /**
   * Fraction of the canvas resolution the scene was rendered at, chosen by
   * dynamic resolution between `minRenderScale` and `renderScale`.
   */
  renderScale?: number;
  /**
   * Tree triangles drawn (meshes, impostors and shadow casters), read
   * back from the GPU a frame or two late.
   */
  treeTriangles?: number;
  /**
   * Milliseconds spent growing the trees in WASM: before the first frame
   * (the first variant of every species), and after it (the rest).
   */
  treeGrowthMs?: [number, number];
  /**
   * Milliseconds from submitting the tree impostor bakes until the GPU
   * finished them, in all.
   */
  treeBakeMs?: number;
}

/**
 * GPU time spent in each render pass, in milliseconds. A pass that did not
 * run reports 0.
 */
export interface GpuPassTimes {
  /** Tree shadow map. */
  shadows: number;
  /** GPU tree culling. */
  treeCulling: number;
  /** Terrain. */
  terrain: number;
  /** Trees: near meshes and distant impostors. */
  trees: number;
  /** Grass. */
  grass: number;
  /** Cloud raymarching, including rain curtains. */
  clouds: number;
  /** Sky, fog, mist, falling rain and snow, and tone mapping. */
  skyAndFog: number;
  /** Ocean, rivers, and lakes. */
  water: number;
  /** Upscaling to the canvas and raindrops on the lens. */
  present: number;
  /**
   * Vegetation streaming: filling tree, grass and boulder tiles near the
   * camera, culling the streamed grass and boulders, and grounding them
   * again when the terrain mesh recentres.
   */
  generation?: number;
  /** With `splitTreeTiming`, the part of `trees` spent on canopy meshes. */
  treeMeshes?: number;
  /** With `splitTreeTiming`, the part of `trees` spent on understorey meshes. */
  understorey?: number;
  /** With `splitTreeTiming`, the part of `trees` spent on impostors. */
  treeImpostors?: number;
  /** Boulders and talus below rock outcrops. */
  boulders?: number;
  /**
   * Stepping the wet ground, puddles and snow under the weather. It runs
   * four times a second, so most frames report 0.
   */
  surfaceWeather?: number;
}

/**
 * Heightmap export options.
 */
export interface ExportHeightmapOptions {
  format?: "float32-le";
}

/**
 * The maps `exportMap()` reads. See `docs/export-and-snapshots.md` for
 * each one's type, channels and units.
 */
export type MapKind =
  | "height"
  | "biome"
  | "water"
  | "waterDepth"
  | "flow"
  | "discharge"
  | "materials"
  | "slope"
  | "normals"
  | "occlusion"
  | "temperature"
  | "moisture"
  | "treeDensity"
  | "grassDensity"
  | "sourceHeight";

/**
 * One entry of a map's legend: a categorical value, or a `materials`
 * channel.
 */
export interface MapLegendEntry {
  index: number;
  name: string;
  /** Display colour, each component 0 to 1. */
  colour: [number, number, number];
}

/**
 * How to read an exported map's values.
 */
export interface MapEncoding {
  /** Units of `value x scale`, such as `"m"` or `"trees/ha"`. */
  units?: string;
  /** A stored value times this is the value in `units`. */
  scale?: number;
  /** Lowest and highest value, for float maps. */
  range?: [number, number];
  /** Names and colours of a categorical map's values or of the channels. */
  legend?: MapLegendEntry[];
  /** Metres between neighbouring pixels along x and z. */
  metresPerPixel?: [number, number];
  /** Sea level in metres. */
  seaLevelMetres?: number;
  /** Where the terrain came from, such as `"vistawasm-fractal-0.2.0"`. */
  generator?: string;
}

/**
 * A map returned by `exportMap()`: `width x height` pixels of `channels`
 * values each, row-major from the terrain's -x, -z corner, channels
 * interleaved.
 */
export interface ExportedMap {
  kind: MapKind;
  width: number;
  height: number;
  channels: number;
  type: "float32" | "uint8" | "uint16";
  data: Float32Array | Uint8Array | Uint16Array;
  encoding: MapEncoding;
}

/**
 * The map the generated engine returns, before the wrapper names its kind
 * and type and gathers its encoding.
 */
export interface RawExportedMap {
  width: number;
  height: number;
  channels: number;
  data: Float32Array | Uint8Array;
  units?: string;
  scale?: number;
  range?: Float32Array;
  /** Legend names, one a line. */
  legendNames: string;
  /** Three colour components a legend entry. */
  legendColours: Float32Array;
  metresPerPixel: Float32Array;
  seaLevelMetres: number;
  generator: string;
}

/**
 * `exportMap()` options.
 */
export interface ExportMapOptions {
  /**
   * Output size `[width, height]`, each a whole number from 2 to 2048, the
   * largest terrain (WebAssembly's limit on terrain size).
   * Defaults to the terrain's own size. Heights and other continuous
   * maps are resampled bicubically, normals and materials bilinearly,
   * and biomes and water by the nearest sample.
   */
  size?: [number, number];
}

/**
 * One exported tree.
 */
export interface TreeRecord {
  /** Trunk base in world metres. */
  x: number;
  y: number;
  z: number;
  species: TreeSpecies;
  /** Which of the species' grown shapes it is drawn with, 0 to 3. */
  variant: number;
  scale: number;
  /** Rotation about the vertical axis in radians. */
  rotation: number;
  /** Colour variation from 0 to 1. */
  tint: number;
  /** Colour dryness from 0 (lush) to 1 (dry). */
  dryness: number;
  /** Set on trees from `setTreeInstances()`. */
  handPlaced?: boolean;
}

/**
 * `exportTrees()` options.
 */
export interface ExportTreesOptions {
  /** The part of the map to export, in world metres. Defaults to all of it. */
  region?: { minX: number; minZ: number; maxX: number; maxZ: number };
  /**
   * Most trees to return, 1 to 10,000,000. Defaults to 2,000,000. More
   * trees than this throws `OPTIONS_INVALID`.
   */
  maxCount?: number;
}

/**
 * The options in effect, from `getOptionsSnapshot()`: the options the
 * engine was created with and every option set since (parts never set
 * are at their defaults), and the fractal options of a generated terrain.
 */
export interface VistaOptionsSnapshot extends VistaEngineOptions {
  /** The options the terrain was generated with; absent for loaded heightmaps. */
  terrain?: FractalTerrainOptions;
  /** The landform a loaded heightmap was given (`RawHeightmapOptions.landform`). */
  landform?: LandformKind;
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
  /**
   * Terrain generation progress from 0 to 1 within each phase.
   * `generateFractal()` reports `"fractal"` at 0 and 1 around the whole
   * call, and in between `"tectonics"`, `"drainage"`, `"detail"`,
   * `"erosion"` (when erosion is requested, at least every 10 %) and
   * `"finishing"` (conditioning the map, then building rivers, flora and
   * the terrain mesh), with `"rivers"` at 0 and 1 around the river build
   * inside it.
   */
  progress: { phase: string; progress: number };
  warning: { message: string; details?: unknown };
  terrainLoaded: TerrainHandle;
  stats: RenderStats;
  fatalError: Error;
  /**
   * The WebGPU device was lost; the error's message gives the browser's
   * reason. Rendering has stopped: dispose of the engine and create a new
   * one to continue.
   */
  deviceLost: Error;
  /**
   * The GPU reported a validation or out-of-memory error outside a call
   * that could reject with it (`code` is `"GPU_ERROR"`).
   */
  gpuError: Error;
  /** The dominant weather changed. */
  weatherChanged: WeatherKind | null;
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
  /** Mean annual temperature in °C at a world position, or null off the terrain. */
  temperatureAt(x: number, z: number): number | null;
  /**
   * Paint rivers and lakes into the terrain, which carves and draws them
   * like its own, or remove them with `null`, which restores the terrain
   * exactly. A mask of another size is resampled, with a `"warning"`
   * event. The mask stays through `setWater()` and is cleared when new
   * terrain loads.
   */
  setWaterMask(mask: WaterMask | null): void;
  /**
   * Load a heightmap image: a PNG at 8 or 16 bits a channel, or any
   * image the browser decodes (at 8 bits, with a `"warning"` event).
   */
  loadHeightmapImage(source: ImageSource, options: HeightmapImageImportOptions): Promise<TerrainHandle>;
  /**
   * Paint biomes onto the terrain, or clear them with `null`. Painted
   * biomes are absolute, with irregular, dithered borders; painted
   * ocean above sea level is classified as usual, with a `"warning"`
   * event. A map of another size is resampled (nearest), with a
   * `"warning"` event. It stays through option changes and clears when
   * new terrain loads.
   */
  setBiomeMap(map: BiomeMap | null): void;
  /**
   * Scale where trees and grass grow with density masks. An omitted key
   * keeps its mask; `null` clears it. Masks of another size are
   * resampled (bilinear), with a `"warning"` event. They stay through
   * option changes and clear when new terrain loads.
   */
  setVegetationMasks(masks: VegetationMasks): void;
  /** The painted biome map, water mask and density masks in effect, as they were set. */
  getPaintedMaps(): PaintedMaps;
  /**
   * The loudest river, waterfall, lake shore and surf near a world
   * position, for your own audio. Cheap enough to call every frame.
   */
  getWaterSounds(x: number, y: number, z: number): WaterSounds;
  /**
   * Every waterfall on the terrain. A cascade of falls close together is
   * listed once, with its total drop, where its lowest fall lands.
   */
  getWaterfalls(): Waterfall[];
  /** The inflows in use, including the one `inflow: "auto"` placed. */
  getInflows(): WaterInflow[];
  setDebugView(debugView: DebugView): void;
  /** Replace weather controls. Changing `state` blends to the new weather. */
  setWeather(weather: WeatherOptions): void;
  /** The current blended weather, or `undefined` when the weather is off. */
  getWeather(): WeatherState | undefined;
  /** Every resolved weather preset, built in and custom, by name. */
  getWeatherPresets(): Record<string, ResolvedWeatherPreset>;
  /**
   * The weather at a world position, for gameplay and audio, or `null`
   * with no terrain. Beyond the terrain the ground values are 0.
   */
  weatherAt(x: number, z: number): LocalWeather | null;
  /** Run the weather forward by 0 to 86400 seconds at once. */
  advanceWeather(seconds: number): void;
  /** Replace the time of day controls. */
  setTimeOfDay(options: TimeOfDayOptions): void;
  /** The time of day and where it puts the sun. */
  getTimeOfDay(): TimeOfDay;
  setShadows(shadows: ShadowOptions): void;
  setSurface(surface: SurfaceOptions): void;
  /** Replace one species' model. Impostors and shadows follow the new model. */
  setTreeModel(species: TreeSpecies, model: TreeModel): void;
  /** Restore one species' procedural model. */
  resetTreeModel(species: TreeSpecies): void;
  /**
   * Replace procedural tree placement with your own trees, or pass
   * `undefined` to restore procedural placement.
   */
  setTreeInstances(trees: TreePlacement[] | undefined): void;
  /**
   * Replace one texture layer with 512 x 512 RGBA8 texels. Use
   * `imageToRgba()` to convert an image.
   */
  replaceTexture(target: TextureTarget, layer: number, rgba: Uint8Array | Uint8ClampedArray): void;
  /** Restore every procedural texture. */
  resetTextures(): void;
  renderOnce(): RenderStats;
  start(): void;
  stop(): void;
  resize(width: number, height: number, devicePixelRatio?: number): void;
  dispose(): void;
  exportHeightmap(options?: ExportHeightmapOptions): Uint8Array<ArrayBuffer>;
  /**
   * Export one of the maps the renderer builds, at the terrain's own size
   * or resampled to `options.size`.
   */
  exportMap(kind: MapKind, options?: ExportMapOptions): ExportedMap;
  /**
   * Export every tree on the map, or in `options.region`: the whole
   * lattice at the current density, streamed trees included, or the
   * hand-placed trees that replace it.
   */
  exportTrees(options?: ExportTreesOptions): TreeRecord[];
  /**
   * The options in effect, as plain JSON, to rebuild the same scene:
   * `createVistaEngine(canvas, snapshot)`, then
   * `generateFractal(snapshot.terrain)` for a generated terrain.
   */
  getOptionsSnapshot(): VistaOptionsSnapshot;
  exportSnapshot(options?: SnapshotOptions): Promise<Blob>;
  on<EventName extends VistaEventName>(
    eventName: EventName,
    listener: VistaEventListener<EventName>
  ): () => void;
}
