import {
  VistaWasmError,
  attachFlyCameraControls,
  biomeMapFromImage,
  createVistaEngine,
  decodePng,
  densityMaskFromImage,
  downloadBlob,
  loadBundle,
  waterMaskFromImage,
  downloadRawHeightmap,
  encodePng,
  encodeRaw,
  exportBundle,
  treesToCsv,
  downloadText,
  exportHeightmapImage,
  exportTerrainObj,
  imageToRgba,
  renderHeightmapToCanvas
} from "@vista-wasm/vista-wasm";
import { MAX_TERRAIN_SIDE, checkFile, checkImage } from "./file-checks.js";
import { createLoadingOverlay } from "./loading.js";
import { createPaintTab } from "./paint/paint-tab.js";
import { overlayForKey } from "./shortcuts.js";

const canvas = document.querySelector("#vista");
const minimap = document.querySelector("#minimap");
const status = document.querySelector("#status");
const statsPanel = document.querySelector("#stats");
const biomeReadout = document.querySelector("#biomeReadout");
const biomeLegend = document.querySelector("#biomeLegend");
const shell = document.querySelector("#shell");
const hint = document.querySelector("#hint");
const panel = document.querySelector("#controls");
const showControlsButton = document.querySelector("#showControls");
const loading = createLoadingOverlay(document.querySelector("#loading"));

const inputs = {
  landform: select("landform"),
  edges: select("edges"),
  seed: input("seed"),
  size: select("size"),
  noiseKind: select("noiseKind"),
  octaves: input("octaves"),
  horizontalScale: input("horizontalScale"),
  verticalScale: input("verticalScale"),
  warp: input("warp"),
  shapeIsland: input("shapeIsland"),
  shapeTerrace: input("shapeTerrace"),
  shapeBasin: input("shapeBasin"),
  shapeCanyon: input("shapeCanyon"),
  shapeCrater: input("shapeCrater"),
  erosionEnabled: input("erosionEnabled"),
  erosionQuality: select("erosionQuality"),
  erosionCustom: input("erosionCustom"),
  hydraulicIterations: input("hydraulicIterations"),
  thermalIterations: input("thermalIterations"),
  sun: input("sun"),
  sunElevation: input("sunElevation"),
  sunIntensity: input("sunIntensity"),
  haze: input("haze"),
  exposure: input("exposure"),
  waterEnabled: input("waterEnabled"),
  sea: input("sea"),
  waveScale: input("waveScale"),
  reflectivity: input("reflectivity"),
  wavesEnabled: input("wavesEnabled"),
  waveAmplitude: input("waveAmplitude"),
  waveLength: input("waveLength"),
  waveDirection: input("waveDirection"),
  waveSteepness: input("waveSteepness"),
  waveSpeed: input("waveSpeed"),
  waveSpread: input("waveSpread"),
  currentSpeed: input("currentSpeed"),
  currentDirection: input("currentDirection"),
  waterClarity: input("waterClarity"),
  waterFoam: input("waterFoam"),
  waterEddies: input("waterEddies"),
  waterRefraction: input("waterRefraction"),
  riversEnabled: input("riversEnabled"),
  riverCatchment: input("riverCatchment"),
  riverWidth: input("riverWidth"),
  riverCurrent: input("riverCurrent"),
  riverSnowmelt: input("riverSnowmelt"),
  riverMeanders: input("riverMeanders"),
  riverMaturity: input("riverMaturity"),
  riverBraiding: input("riverBraiding"),
  riverSprings: input("riverSprings"),
  riverWaterfalls: input("riverWaterfalls"),
  riverInflow: input("riverInflow"),
  riverInflowDischarge: input("riverInflowDischarge"),
  riverRiparian: input("riverRiparian"),
  waterReflections: input("waterReflections"),
  biomesEnabled: input("biomesEnabled"),
  biomeTemperature: input("biomeTemperature"),
  biomeMoisture: input("biomeMoisture"),
  biomeScale: input("biomeScale"),
  biomeVolcanism: input("biomeVolcanism"),
  biomeBeachHeight: input("biomeBeachHeight"),
  biomeSnowLineAuto: input("biomeSnowLineAuto"),
  biomeSnowLine: input("biomeSnowLine"),
  biomeCelsiusAuto: input("biomeCelsiusAuto"),
  biomeCelsius: input("biomeCelsius"),
  floraEnabled: input("floraEnabled"),
  floraDensity: input("floraDensity"),
  treeLine: input("treeLine"),
  treeQuality: select("treeQuality"),
  speciesVariation: input("speciesVariation"),
  windStrength: input("windStrength"),
  meshDistance: input("meshDistance"),
  treeVariants: input("treeVariants"),
  grassEnabled: input("grassEnabled"),
  grassStyle: select("grassStyle"),
  grassDensity: input("grassDensity"),
  grassViewDistance: input("grassViewDistance"),
  grassForestFloor: input("grassForestFloor"),
  cloudStyle: select("cloudStyle"),
  cloudCoverage: input("cloudCoverage"),
  cloudSpeed: input("cloudSpeed"),
  cloudHeight: input("cloudHeight"),
  cloudWindDirection: input("cloudWindDirection"),
  cloudEvolution: input("cloudEvolution"),
  cloudThickness: input("cloudThickness"),
  cloudDensity: input("cloudDensity"),
  cloudShadows: input("cloudShadows"),
  cloudTemporal: input("cloudTemporal"),
  cloudCirrus: input("cloudCirrus"),
  cloudCirrusHeight: input("cloudCirrusHeight"),
  cloudCirrusSpeed: input("cloudCirrusSpeed"),
  cloudSteps: select("cloudSteps"),
  cloudResolution: input("cloudResolution"),
  cloudStratiform: input("cloudStratiform"),
  cloudTowering: input("cloudTowering"),
  cloudBaseDarkness: input("cloudBaseDarkness"),
  cloudRaggedBase: input("cloudRaggedBase"),
  cloudRainShafts: input("cloudRainShafts"),
  cloudBaseVariation: input("cloudBaseVariation"),
  cloudBaseLumpiness: input("cloudBaseLumpiness"),
  cloudAltocumulus: input("cloudAltocumulus"),
  cloudAltostratus: input("cloudAltostratus"),
  cloudAltoHeight: input("cloudAltoHeight"),
  cloudAltoSpeed: input("cloudAltoSpeed"),
  mistStyle: select("mistStyle"),
  mistDensity: input("mistDensity"),
  mistBaseHeight: input("mistBaseHeight"),
  mistHeightFalloff: input("mistHeightFalloff"),
  mistRiseAboveWater: input("mistRiseAboveWater"),
  mistWindSpeed: input("mistWindSpeed"),
  mistWindDirection: input("mistWindDirection"),
  mistSunScattering: input("mistSunScattering"),
  weatherEnabled: input("weatherEnabled"),
  weatherState: select("weatherState"),
  weatherAutoCycle: input("weatherAutoCycle"),
  weatherAllowSnow: input("weatherAllowSnow"),
  weatherTransition: input("weatherTransition"),
  weatherWindScale: input("weatherWindScale"),
  weatherPrecipitation: input("weatherPrecipitation"),
  weatherLensDrops: input("weatherLensDrops"),
  weatherLensDropCount: input("weatherLensDropCount"),
  weatherLensDropMin: input("weatherLensDropMin"),
  weatherLensDropMax: input("weatherLensDropMax"),
  weatherDuration: input("weatherDuration"),
  weatherEffectClouds: input("weatherEffectClouds"),
  weatherEffectMist: input("weatherEffectMist"),
  weatherEffectWind: input("weatherEffectWind"),
  weatherEffectWater: input("weatherEffectWater"),
  weatherEffectPrecipitation: input("weatherEffectPrecipitation"),
  weatherEffectGround: input("weatherEffectGround"),
  weatherEffectLightning: input("weatherEffectLightning"),
  weatherRegional: input("weatherRegional"),
  presetCoverage: input("presetCoverage"),
  presetHumidity: input("presetHumidity"),
  presetTurbidity: input("presetTurbidity"),
  presetWind: input("presetWind"),
  presetRain: input("presetRain"),
  presetSnow: input("presetSnow"),
  presetCoverageSpread: input("presetCoverageSpread"),
  presetPrecipitationSpread: input("presetPrecipitationSpread"),
  timeOfDayEnabled: input("timeOfDayEnabled"),
  timeOfDayHours: input("timeOfDayHours"),
  timeOfDayLength: input("timeOfDayLength"),
  timeOfDayLatitude: input("timeOfDayLatitude"),
  terrainShadows: input("terrainShadows"),
  terrainShadowSoftness: input("terrainShadowSoftness"),
  treeShadows: input("treeShadows"),
  treeShadowDistance: input("treeShadowDistance"),
  treeShadowResolution: select("treeShadowResolution"),
  treeShadowSoftness: input("treeShadowSoftness"),
  cloudShadowsEnabled: input("cloudShadowsEnabled"),
  cloudShadowStrength: input("cloudShadowStrength"),
  shadowStrength: input("shadowStrength"),
  surfaceTextures: input("surfaceTextures"),
  surfaceNormals: input("surfaceNormals"),
  surfaceTextureScale: input("surfaceTextureScale"),
  surfaceRockiness: input("surfaceRockiness"),
  surfaceBoulders: input("surfaceBoulders"),
  surfaceBoulderDistance: input("surfaceBoulderDistance"),
  replaceTarget: select("replaceTarget"),
  replaceFile: input("replaceFile"),
  palmBeaches: input("palmBeaches"),
  groundGrove: input("groundGrove"),
  quality: select("quality"),
  maxFrameRate: select("maxFrameRate"),
  renderScale: select("renderScale"),
  dynamicResolution: input("dynamicResolution"),
  minRenderScale: select("minRenderScale"),
  renderDistance: select("renderDistance"),
  vegetationDetail: select("vegetationDetail"),
  canopyDistance: select("canopyDistance"),
  maxTreeTriangles: select("maxTreeTriangles"),
  grassDetail: select("grassDetail"),
  splitTreeTiming: input("splitTreeTiming"),
  detailDistance: select("detailDistance"),
  cloudDistance: select("cloudDistance"),
  renderFade: select("renderFade"),
  cloudFade: select("cloudFade"),
  debugView: select("debugView"),
  exportKind: select("exportKind"),
  exportSize: select("exportSize"),
  exportFormat: select("exportFormat"),
  importHeight: input("importHeight"),
  importMetres: input("importMetres"),
  importMin: input("importMin"),
  importMax: input("importMax"),
  importBiome: input("importBiome"),
  importWater: input("importWater"),
  importTrees: input("importTrees"),
  importGrass: input("importGrass"),
  importBundle: input("importBundle")
};

const buttons = {
  generate: button("generate"),
  screenshot: button("screenshot"),
  downloadMap: button("downloadMap"),
  downloadModel: button("downloadModel"),
  downloadHeightmap: button("downloadHeightmap"),
  exportMap: button("exportMap"),
  exportTrees: button("exportTrees"),
  exportBundle: button("exportBundle"),
  loadHeightImage: button("loadHeightImage"),
  clearPainted: button("clearPainted"),
  resetTextures: button("resetTextures"),
  customModel: button("customModel"),
  plantGrove: button("plantGrove"),
  treeShowcase: button("treeShowcase"),
  jumpToWaterfall: button("jumpToWaterfall"),
  riverValley: button("riverValley"),
  meanderingLowland: button("meanderingLowland"),
  braidedValley: button("braidedValley"),
  deltaCoast: button("deltaCoast"),
  jumpToMainRiver: button("jumpToMainRiver"),
  weatherSkip: button("weatherSkip"),
  presetReset: button("presetReset"),
  mackerelSky: button("mackerelSky"),
  veiledSun: button("veiledSun")
};

// Where the "At the camera" inflow enters, in world x and z.
let inflowAt = [0, 0];
// Whether the inflow came with a preset, which places it for its own map.
let presetInflow = false;

let customModelActive = false;
let groveActive = false;
// The point the current grove is planted around, so it can be planted
// again when the grounding checkbox changes.
let groveCentre = null;
let lastCamera = null;

const weatherReadout = document.querySelector("#weatherReadout");
const timeOfDayReadout = document.querySelector("#timeOfDayReadout");
const WEATHER_NAMES = {
  clear: "clear",
  fewClouds: "few clouds",
  partlyCloudy: "partly cloudy",
  brokenClouds: "broken clouds",
  overcast: "overcast",
  mist: "mist",
  fog: "fog",
  lightRain: "light rain",
  rain: "rain",
  heavyRain: "heavy rain",
  storm: "storm",
  snow: "snow",
  blizzard: "blizzard"
};

// Custom presets have no friendly name, so they show their own.
function weatherName(kind) {
  return WEATHER_NAMES[kind] ?? kind;
}

// The preset editor's sliders and the preset field each one sets.
const PRESET_FIELDS = [
  ["presetCoverage", "cloudCoverage"],
  ["presetHumidity", "humidity"],
  ["presetTurbidity", "turbidity"],
  ["presetWind", "windMetresPerSecond"],
  ["presetRain", "rain"],
  ["presetSnow", "snow"],
  ["presetCoverageSpread", "coverageSpread"],
  ["presetPrecipitationSpread", "precipitationSpread"]
];

// Edited presets, by name, sent as `WeatherOptions.presets`.
const presetOverrides = {};

let engine = null;
// The fly camera, detached while the Paint tab is open.
let controls = null;
let paintTab = null;
let latestMetadata = null;
let latestHeights = null;
let minimapBaseImage = null;

function input(id) {
  const element = document.querySelector(`#${id}`);

  if (!element) {
    throw new Error(`Missing #${id} control.`);
  }

  return element;
}

function select(id) {
  const element = document.querySelector(`#${id}`);

  if (!element) {
    throw new Error(`Missing #${id} control.`);
  }

  return element;
}

function button(id) {
  const element = document.querySelector(`#${id}`);

  if (!element) {
    throw new Error(`Missing #${id} control.`);
  }

  return element;
}

function readNumber(element, fallback) {
  const value = Number(element.value);
  return Number.isFinite(value) ? value : fallback;
}

// The weather where the camera is: its own part of the regional weather.
function localWeatherLines() {
  const weather = engine?.getWeather();

  if (!weather) {
    return [];
  }

  const percent = (value) => `${Math.round(value * 100)} %`;
  return [
    `Here: cloud ${percent(weather.cloudCoverage)}, rain and snow ${percent(Math.min(weather.rain + weather.snow, 1))}`,
    `Ground: wet ${percent(weather.wetness)}, puddles ${percent(weather.puddles ?? 0)}, snow ${percent(weather.snowCover)}`
  ];
}

// Where each frame's GPU time goes, largest first, so the pass worth
// making cheaper is at the top.
function gpuProfile(stats) {
  const times = stats.gpuPassTimesMs;

  if (!times) {
    return ["GPU timing: not available in this browser"];
  }

  const names = {
    terrain: "Terrain",
    trees: "Trees",
    grass: "Grass",
    boulders: "Boulders",
    clouds: "Clouds",
    skyAndFog: "Sky and fog",
    water: "Water",
    shadows: "Shadows",
    treeCulling: "Tree culling",
    present: "Upscale and lens",
    surfaceWeather: "Wet ground"
  };
  // With split tree timing, the trees row breaks down into its parts.
  const parts = {
    treeMeshes: "canopy meshes",
    understorey: "understorey",
    treeImpostors: "impostors"
  };
  const rows = Object.entries(names)
    .map(([key, name]) => [key, name, times[key]])
    .filter(([, , ms]) => ms > 0.005)
    .sort((a, b) => b[2] - a[2])
    .flatMap(([key, name, ms]) => [
      `  ${name} ${ms.toFixed(2)} ms`,
      ...(key === "trees"
        ? Object.entries(parts)
          .filter(([part]) => (times[part] ?? 0) > 0.005)
          .map(([part, label]) => `    ${label} ${times[part].toFixed(2)} ms`)
        : [])
    ]);
  const triangles = stats.treeTriangles
    ? [`Tree triangles ${(stats.treeTriangles / 1e6).toFixed(2)} million`]
    : [];
  return [`GPU ${(stats.gpuFrameTimeMs ?? 0).toFixed(2)} ms per frame`, ...rows, ...triangles];
}

const WATER_SOUND_NAMES = {
  river: "river",
  waterfall: "waterfall",
  lakeShore: "lake shore",
  surf: "surf"
};

// The nearest water the camera can hear, and how loud it is.
function describeWaterSounds(sounds) {
  let nearest = null;

  for (const [kind, sound] of Object.entries(sounds)) {
    if (sound && (!nearest || sound.distanceMetres < nearest.sound.distanceMetres)) {
      nearest = { kind, sound };
    }
  }

  if (!nearest) {
    return "none within earshot";
  }

  const { kind, sound } = nearest;
  return `${WATER_SOUND_NAMES[kind]} ${Math.round(sound.distanceMetres)} m away, loudness ${sound.loudness.toFixed(2)}`;
}

// Put an explicit inflow where the camera stands, kept on the map.
const LOWLAND_SIZE = 512;

// Heights for the Meandering lowland preset, 30 m apart: a valley 15 km
// long falling 0.1 % from west to east into the sea, its floor 900 m
// wide, between walls rising 20 m to a plateau of rolling hills (a value
// noise of four octaves, so streams on it gather into valleys) that
// rises gently away from the valley.
function lowlandHeights() {
  const heights = new Float32Array(LOWLAND_SIZE * LOWLAND_SIZE);
  // The axis lies on a row of samples: between two rows, both would drain
  // side by side as two channels.
  const middle = LOWLAND_SIZE / 2;

  for (let y = 0; y < LOWLAND_SIZE; y += 1) {
    for (let x = 0; x < LOWLAND_SIZE; x += 1) {
      const out = Math.max(0, Math.abs(y - middle) - 15);
      let hills = 0;

      for (let octave = 0; octave < 4; octave += 1) {
        hills += latticeNoise((x / 48) * 2 ** octave, (y / 48) * 2 ** octave) * 30 * 0.5 ** octave;
      }

      // The floor also falls gently to the valley's axis, as floodplains do,
      // so rain on it finds the river instead of running beside it.
      const floor =
        15 +
        (LOWLAND_SIZE - 11 - x) * 30 * 0.001 -
        Math.max(0, x - (LOWLAND_SIZE - 11)) * 2 +
        Math.min(Math.abs(y - middle), 15) * 0.1;
      // The plateau rises gently away from the valley, so it drains into it
      // rather than into flat basins.
      heights[y * LOWLAND_SIZE + x] = floor + Math.min(out, 10) * 2 + out * 0.12 + hills * Math.min(1, out / 20);
    }
  }

  return heights;
}

// A value noise on a lattice of whole numbers, smoothed between them:
// the presets' hills and ranges.
function latticeNoise(x, y) {
  const hash = (i, j) => {
    const h = Math.sin(i * 127.1 + j * 311.7) * 43758.5453;
    return h - Math.floor(h);
  };
  const smooth = (t) => t * t * (3 - 2 * t);
  const [ix, iy] = [Math.floor(x), Math.floor(y)];
  const [tx, ty] = [smooth(x - ix), smooth(y - iy)];
  const top = hash(ix, iy) + (hash(ix + 1, iy) - hash(ix, iy)) * tx;
  const bottom = hash(ix, iy + 1) + (hash(ix + 1, iy + 1) - hash(ix, iy + 1)) * tx;
  return top + (bottom - top) * ty;
}

// Heights for the Braided valley preset, 20 m apart and never under the
// sea, so the river leaves by the map's eastern edge: a glacial outwash
// plain 1.6 km wide falling 1 % from west to east, steep enough and wide
// enough for its river to braid (a channel braids only where it is at
// least a sample wide), between mountain walls that rise to ridged ranges
// about 1,000 m above it.
const BRAIDED_METRES = 20;
function braidedHeights() {
  const heights = new Float32Array(LOWLAND_SIZE * LOWLAND_SIZE);
  const middle = LOWLAND_SIZE / 2;

  for (let y = 0; y < LOWLAND_SIZE; y += 1) {
    for (let x = 0; x < LOWLAND_SIZE; x += 1) {
      const out = Math.max(0, Math.abs(y - middle) - 40);
      let ridges = 0;

      for (let octave = 0; octave < 5; octave += 1) {
        const n = latticeNoise((x / 64) * 2 ** octave, (y / 64) * 2 ** octave);
        ridges += (1 - Math.abs(2 * n - 1)) * 0.5 ** octave;
      }

      // The plain sags a little to its axis, so its braids stay on it.
      const plain = 200 + (LOWLAND_SIZE - x) * BRAIDED_METRES * 0.01 + Math.min(Math.abs(y - middle), 40) * 0.1;
      const wall = Math.min(out / 60, 1);
      heights[y * LOWLAND_SIZE + x] = plain + out * 6 * (1 - 0.5 * wall) + ridges * 900 * wall;
    }
  }

  return heights;
}

// Heights for the Delta coast preset, 30 m apart: rolling hills falling
// to a coastal plain 6 km wide that slopes 0.15 % to the sea, which
// shelves gently, so a big river reaching it builds a delta.
function deltaHeights() {
  const heights = new Float32Array(LOWLAND_SIZE * LOWLAND_SIZE);
  const middle = LOWLAND_SIZE / 2;
  const coast = 360;

  for (let y = 0; y < LOWLAND_SIZE; y += 1) {
    for (let x = 0; x < LOWLAND_SIZE; x += 1) {
      let hills = 0;

      for (let octave = 0; octave < 4; octave += 1) {
        hills += latticeNoise((x / 48) * 2 ** octave, (y / 48) * 2 ** octave) * 0.5 ** octave;
      }

      const inland = Math.max(0, 160 - x);
      // The plain falls to the coast and gently to the river's line.
      const plain = 1.5 + (coast - x) * 30 * 0.0015 + Math.min(Math.abs(y - middle), 60) * 0.02;
      const sea = -Math.min((x - coast) * 0.06, 20);
      const ground = x < coast ? plain + inland * 0.6 + hills * 25 * Math.min(1, inland / 40) : sea;
      heights[y * LOWLAND_SIZE + x] = ground;
    }
  }

  return heights;
}

function placeInflowAtCamera(controls) {
  const [x, , z] = controls.getCamera().position;
  const metadata = latestMetadata;
  const halfX = metadata ? ((metadata.width - 1) * metadata.metresPerSample) / 2 : 0;
  const halfZ = metadata ? ((metadata.height - 1) * metadata.metresPerSample) / 2 : 0;
  inflowAt = [Math.max(-halfX, Math.min(halfX, x)), Math.max(-halfZ, Math.min(halfZ, z))];
  presetInflow = false;
}

// Follow the carved ground downhill from the first inflow for about a
// kilometre, then look down the river from 120 m above it.
function jumpToMainRiver(controls) {
  const inflows = engine?.getInflows() ?? [];

  if (inflows.length === 0 || !latestMetadata) {
    setStatus("No river enters this map. Use open edges and an automatic inflow, or the River valley preset.");
    return;
  }

  const step = latestMetadata.metresPerSample;
  let [x, , z] = inflows[0].position;
  const path = [[x, z]];

  for (let walked = 0; walked < 1400; walked += step) {
    let best = null;

    for (const [dx, dz] of [[1, 0], [-1, 0], [0, 1], [0, -1], [1, 1], [1, -1], [-1, 1], [-1, -1]]) {
      const nx = x + dx * step;
      const nz = z + dz * step;
      const height = terrainHeightAt(nx, nz);

      if (height !== null && !path.some(([px, pz]) => px === nx && pz === nz) && (!best || height < best.height)) {
        best = { height, nx, nz };
      }
    }

    if (!best) {
      break;
    }

    x = best.nx;
    z = best.nz;
    path.push([x, z]);
  }

  const at = path[Math.max(0, path.length - 8)];
  const ahead = path[path.length - 1];
  controls.setPosition([at[0], (terrainHeightAt(at[0], at[1]) ?? 0) + 120, at[1]]);
  controls.lookAt([ahead[0], terrainHeightAt(ahead[0], ahead[1]) ?? 0, ahead[1]]);
  setStatus(`A river of ${inflows[0].dischargeCubicMetresPerSecond.toFixed(0)} m³/s enters the map here.`);
}

// Bring the camera back when new terrain is smaller than the last map
// and leaves it beyond the edge: 150 m above the map, a third of the way
// in from one side, looking across to its middle.
function keepCameraOnMap(controls) {
  if (!latestMetadata) {
    return;
  }

  const { width, height, metresPerSample: step } = latestMetadata;
  const halfX = ((width - 1) * step) / 2;
  const halfZ = ((height - 1) * step) / 2;
  const [x, , z] = controls.getCamera().position;

  if (Math.abs(x) <= halfX && Math.abs(z) <= halfZ) {
    return;
  }

  const sea = readNumber(inputs.sea, 0);
  const from = halfZ / 3;
  controls.setPosition([0, Math.max(terrainHeightAt(0, from) ?? sea, sea) + 150, from]);
  controls.lookAt([0, Math.max(terrainHeightAt(0, 0) ?? sea, sea), 0]);
}

// Lift the camera clear of the ground when new terrain puts it inside a
// hill, keeping where it looks.
function keepCameraAboveGround(controls) {
  const camera = controls.getCamera();
  const [x, y, z] = camera.position;
  const ground = terrainHeightAt(x, z);

  if (ground !== null && y < ground + 2) {
    const rise = ground + 30 - y;
    controls.setPosition([x, y + rise, z]);
    controls.lookAt([camera.target[0], camera.target[1] + rise, camera.target[2]]);
  }
}

// A close view of the largest river on the map where it runs gently: from
// the water's edge at eye height, looking down the river as it runs on,
// with a late-afternoon sun behind the camera and to one side. Returns
// whether a river was found.
function viewRiver(controls) {
  if (!engine || !latestMetadata || !latestHeights) {
    return false;
  }

  const { width, height, metresPerSample: step } = latestMetadata;
  const water = engine.exportMap("water").data;
  const discharge = engine.exportMap("discharge").data;
  const sea = readNumber(inputs.sea, 0);
  const world = (column, row) => [
    (column - (width - 1) / 2) * step,
    (row - (height - 1) / 2) * step
  ];
  const ground = (x, z) => terrainHeightAt(x, z) ?? sea;
  const river = (column, row) =>
    column >= 0 && row >= 0 && column < width && row < height && water[row * width + column] === 1;
  const margin = 12;
  let best = null;

  for (let row = margin; row < height - margin; row += 1) {
    for (let column = margin; column < width - margin; column += 1) {
      const index = row * width + column;

      if (!river(column, row) || (best && discharge[index] <= best.discharge)) {
        continue;
      }

      const [x, z] = world(column, row);
      const slope =
        Math.hypot(ground(x + step, z) - ground(x - step, z), ground(x, z + step) - ground(x, z - step)) / (2 * step);

      if (ground(x, z) > sea + 1 && slope < 0.06) {
        best = { column, row, discharge: discharge[index] };
      }
    }
  }

  if (!best) {
    return false;
  }

  // Follow the river downstream, where its discharge only grows, to aim
  // along the course it really takes.
  const widthMetres = 2.7 * Math.sqrt(best.discharge);
  const depth = 0.35 * best.discharge ** 0.4;
  const look = Math.max(6 * widthMetres, 80);
  const path = [[best.column, best.row]];
  const seen = new Set([best.row * width + best.column]);

  while (path.length * step < look) {
    const [column, row] = path[path.length - 1];
    let next = null;

    for (let dz = -1; dz <= 1; dz += 1) {
      for (let dx = -1; dx <= 1; dx += 1) {
        const index = (row + dz) * width + column + dx;

        if (river(column + dx, row + dz) && !seen.has(index) && (!next || discharge[index] > next.discharge)) {
          next = { column: column + dx, row: row + dz, discharge: discharge[index] };
        }
      }
    }

    if (!next) {
      break;
    }

    seen.add(next.row * width + next.column);
    path.push([next.column, next.row]);
  }

  if (path.length < 3) {
    return false;
  }

  const [cx, cz] = world(best.column, best.row);
  const [ax, az] = world(...path[Math.min(path.length - 1, Math.ceil((2 * widthMetres) / step) + 1)]);
  const length = Math.hypot(ax - cx, az - cz) || 1;
  const along = [(ax - cx) / length, (az - cz) / length];
  const across = [-along[1], along[0]];
  const level = ground(cx, cz) + depth;
  // At the water's edge, a little upstream, so the river runs away from
  // the camera between its banks.
  const x = cx - along[0] * 1.5 * widthMetres + across[0] * 0.3 * widthMetres;
  const z = cz - along[1] * 1.5 * widthMetres + across[1] * 0.3 * widthMetres;
  const [tx, tz] = world(...path[path.length - 1]);
  controls.setPosition([x, Math.max(level, ground(x, z)) + 2.2, z]);
  controls.lookAt([tx, ground(tx, tz) + depth, tz]);
  // The sun behind the camera and to one side, low and warm.
  const behind = (Math.atan2(-along[1], -along[0]) * 180) / Math.PI + 40;
  inputs.sun.value = String(Math.round(((behind % 360) + 360) % 360));
  inputs.sunElevation.value = "24";
  inputs.sun.dispatchEvent(new Event("input"));
  inputs.sunElevation.dispatchEvent(new Event("input"));
  return true;
}

// One tree of each species in a row on the flattest dry ground near the
// camera, standing on the drawn terrain, framed from 45 m, to judge the
// trees side by side. Procedural trees are hidden until restored.
function showTrees(controls) {
  if (!engine) {
    return;
  }

  const species = ["oak", "pine", "spruce", "palm", "jungle", "cypress", "acacia", "shrub"];
  const from = controls.getCamera().position;
  const sea = readNumber(inputs.sea, 0);
  let best = null;

  for (let radius = 0; radius <= 600 && !best; radius += 30) {
    for (let angle = 0; angle < Math.PI * 2; angle += 0.4) {
      const x = from[0] + Math.cos(angle) * radius;
      const z = from[2] + Math.sin(angle) * radius;
      const heights = [-60, -30, 0, 30, 60].map((dx) => terrainHeightAt(x + dx, z));

      if (heights.some((height) => height === null || height <= sea + 1)) {
        continue;
      }

      const spread = Math.max(...heights) - Math.min(...heights);

      if (spread < 6) {
        best = [x, z];
        break;
      }
    }
  }

  if (!best) {
    setStatus("Fly over flatter dry land to see the tree showcase.");
    return;
  }

  const [x, z] = best;
  const trees = species.map((name, index) => ({
    x: x + (index - 3.5) * 16,
    y: terrainHeightAt(x + (index - 3.5) * 16, z) ?? 0,
    z,
    species: name,
    ground: true
  }));
  engine.setTreeInstances(trees);
  groveActive = true;
  groveCentre = null;
  buttons.plantGrove.textContent = "Restore procedural trees";
  const ground = terrainHeightAt(x, z + 45) ?? trees[3].y;
  controls.setPosition([x, Math.max(ground, trees[3].y) + 6, z + 45]);
  controls.lookAt([x, trees[3].y + 8, z]);
  setStatus("One tree of each species: oak, pine, spruce, palm, jungle, cypress, acacia and shrub.");
}

// Fly to the waterfall nearest the camera and look at it from three times
// its height away.
function jumpToWaterfall(controls) {
  const falls = engine?.getWaterfalls() ?? [];

  if (falls.length === 0) {
    setStatus("This terrain has no waterfalls. Try a steeper landform, or turn waterfalls on.");
    return;
  }

  const from = controls.getCamera().position;
  const distance = (fall) => Math.hypot(fall.position[0] - from[0], fall.position[2] - from[2]);
  const nearest = falls.reduce((best, fall) => (distance(fall) < distance(best) ? fall : best));
  const [x, y, z] = nearest.position;
  const height = nearest.heightMetres;
  const away = Math.max(distance(nearest), 0.001);
  const back = Math.max(height * 3, 15);
  const cameraX = x + ((from[0] - x) / away) * back;
  const cameraZ = z + ((from[2] - z) / away) * back;
  const ground = terrainHeightAt(cameraX, cameraZ) ?? y;
  controls.setPosition([cameraX, Math.max(y + height * 0.5, ground + 3), cameraZ]);
  controls.lookAt([x, y + height * 0.4, z]);
  setStatus(
    `Waterfall ${height.toFixed(0)} m high and ${nearest.widthMetres.toFixed(1)} m wide, ` +
      `carrying ${nearest.dischargeCubicMetresPerSecond.toFixed(2)} m³/s.`
  );
}

function setStatus(message) {
  if (status) {
    status.textContent = message;
  }
}

function showError(error) {
  // Errors must stay visible, so bring back a hidden status line for them
  // without changing the saved choice.
  if (status?.hidden) {
    showOverlay("status", true);
  }

  let message = error instanceof Error ? error.message : "VistaWASM failed.";

  if (error instanceof VistaWasmError) {
    message = `${error.code}: ${error.message}`;
  }

  setStatus(message);
  loading.fail(message);
}

function buildFractalOptions() {
  // Unset iteration counts follow the quality preset, and the landform
  // supplies rain and talus angle.
  const erosion = inputs.erosionEnabled.checked
    ? {
        quality: inputs.erosionQuality.value,
        ...(inputs.erosionCustom.checked
          ? {
              hydraulicIterations: readNumber(inputs.hydraulicIterations, 200),
              thermalIterations: readNumber(inputs.thermalIterations, 100)
            }
          : {})
      }
    : undefined;

  return {
    landform: inputs.landform.value,
    edges: inputs.edges.value,
    seed: readNumber(inputs.seed, 12345),
    size: readNumber(inputs.size, 512),
    horizontalScaleMetres: readNumber(inputs.horizontalScale, 12),
    verticalScale: readNumber(inputs.verticalScale, 1),
    seaLevelMetres: readNumber(inputs.sea, 0),
    noise: {
      kind: inputs.noiseKind.value,
      octaves: readNumber(inputs.octaves, 7),
      gain: 0.52,
      lacunarity: 2.05,
      warp: readNumber(inputs.warp, 0.15)
    },
    shape: {
      island: readNumber(inputs.shapeIsland, 0),
      terrace: readNumber(inputs.shapeTerrace, 0),
      basin: readNumber(inputs.shapeBasin, 0),
      canyon: readNumber(inputs.shapeCanyon, 0),
      crater: readNumber(inputs.shapeCrater, 0)
    },
    erosion
  };
}

function applySun() {
  engine?.setSun({
    azimuthDegrees: readNumber(inputs.sun, 132),
    elevationDegrees: readNumber(inputs.sunElevation, 30),
    intensity: readNumber(inputs.sunIntensity, 1.3)
  });
}

function applyAtmosphere() {
  engine?.setAtmosphere({
    rayleighStrength: 1,
    mieStrength: 0.45,
    hazeDistanceMetres: readNumber(inputs.haze, 60000),
    exposure: readNumber(inputs.exposure, 1.1),
    skyTint: [1, 1, 1]
  });
}

function applyWater() {
  engine?.setWater({
    enabled: inputs.waterEnabled.checked,
    seaLevelMetres: readNumber(inputs.sea, 0),
    waveScale: readNumber(inputs.waveScale, 0.8),
    reflectivity: readNumber(inputs.reflectivity, 0.4),
    shorelineSoftnessMetres: 6,
    waves: {
      enabled: inputs.wavesEnabled.checked,
      amplitudeMetres: readNumber(inputs.waveAmplitude, 0.9),
      wavelengthMetres: readNumber(inputs.waveLength, 38),
      directionDegrees: readNumber(inputs.waveDirection, 35),
      steepness: readNumber(inputs.waveSteepness, 0.55),
      speed: readNumber(inputs.waveSpeed, 1),
      directionalSpread: readNumber(inputs.waveSpread, 0.55)
    },
    rivers: {
      enabled: inputs.riversEnabled.checked,
      minCatchmentKm2: readNumber(inputs.riverCatchment, 0.15),
      widthScale: readNumber(inputs.riverWidth, 1),
      currentSpeed: readNumber(inputs.riverCurrent, 1),
      snowmelt: readNumber(inputs.riverSnowmelt, 1),
      springs: inputs.riverSprings.checked,
      meanders: readNumber(inputs.riverMeanders, 0.6),
      meanderMaturity: readNumber(inputs.riverMaturity, 0.5),
      braiding: readNumber(inputs.riverBraiding, 1),
      waterfalls: inputs.riverWaterfalls.checked,
      inflow:
        inputs.riverInflow.value === "camera"
          ? [{ position: inflowAt, dischargeCubicMetresPerSecond: readNumber(inputs.riverInflowDischarge, 40) }]
          : inputs.riverInflow.value,
      riparian: readNumber(inputs.riverRiparian, 1)
    },
    reflections: inputs.waterReflections.value,
    currentSpeed: readNumber(inputs.currentSpeed, 0.35),
    currentDirectionDegrees: readNumber(inputs.currentDirection, 60),
    clarityMetres: readNumber(inputs.waterClarity, 6),
    foam: readNumber(inputs.waterFoam, 0.7),
    eddies: readNumber(inputs.waterEddies, 1),
    refraction: readNumber(inputs.waterRefraction, 1)
  });
}

function applyBiomes() {
  engine?.setBiomes({
    enabled: inputs.biomesEnabled.checked,
    temperatureBias: readNumber(inputs.biomeTemperature, 0),
    moistureBias: readNumber(inputs.biomeMoisture, 0),
    climateScaleMetres: readNumber(inputs.biomeScale, 7000),
    volcanism: readNumber(inputs.biomeVolcanism, 0.35),
    beachHeightMetres: readNumber(inputs.biomeBeachHeight, 5),
    snowLineMetres: inputs.biomeSnowLineAuto.checked
      ? undefined
      : readNumber(inputs.biomeSnowLine, 1400),
    meanTemperatureCelsius: inputs.biomeCelsiusAuto.checked
      ? undefined
      : readNumber(inputs.biomeCelsius, 15)
  });
}

function applyFlora() {
  engine?.setFlora({
    enabled: inputs.floraEnabled.checked,
    density: readNumber(inputs.floraDensity, 0.35),
    treeLineMetres: readNumber(inputs.treeLine, 1800),
    seedOffset: 3001,
    maxInstances: 500000,
    treeQuality: inputs.treeQuality.value,
    speciesVariation: readNumber(inputs.speciesVariation, 0.6),
    windStrength: readNumber(inputs.windStrength, 0.3),
    meshDistanceMetres: readNumber(inputs.meshDistance, 420),
    variantsPerSpecies: readNumber(inputs.treeVariants, 4),
    speciesRules: inputs.palmBeaches.checked
      ? [
          { biome: "coastalBeach", species: [{ species: "palm", weight: 1 }], density: 1.5 },
          { biome: "coastalRocky", species: [{ species: "palm", weight: 2 }, { species: "shrub", weight: 1 }] }
        ]
      : []
  });
}

function applyGrass() {
  engine?.setGrass({
    enabled: inputs.grassEnabled.checked,
    style: inputs.grassStyle.value,
    density: readNumber(inputs.grassDensity, 0.5),
    viewDistanceMetres: readNumber(inputs.grassViewDistance, 220),
    seedOffset: 7331,
    maxInstances: 1000000,
    forestFloor: inputs.grassForestFloor.checked
  });
}

function applyClouds() {
  engine?.setClouds({
    style: inputs.cloudStyle.value,
    coverage: readNumber(inputs.cloudCoverage, 0.45),
    speed: readNumber(inputs.cloudSpeed, 1),
    heightMetres: readNumber(inputs.cloudHeight, 1800),
    colour: [1, 1, 1],
    seedOffset: 9007,
    raymarchSteps: readNumber(inputs.cloudSteps, 32),
    windDirectionDegrees: readNumber(inputs.cloudWindDirection, 70),
    evolution: readNumber(inputs.cloudEvolution, 0.35),
    thicknessMetres: readNumber(inputs.cloudThickness, 1600),
    density: readNumber(inputs.cloudDensity, 0.6),
    castShadows: inputs.cloudShadows.checked,
    temporal: inputs.cloudTemporal.checked,
    cirrus: readNumber(inputs.cloudCirrus, 0.35),
    cirrusHeightMetres: readNumber(inputs.cloudCirrusHeight, 9000),
    cirrusSpeed: readNumber(inputs.cloudCirrusSpeed, 0.4),
    resolutionScale: readNumber(inputs.cloudResolution, 0.5),
    stratiform: readNumber(inputs.cloudStratiform, 0),
    towering: readNumber(inputs.cloudTowering, 0),
    baseDarkness: readNumber(inputs.cloudBaseDarkness, 0),
    raggedBase: readNumber(inputs.cloudRaggedBase, 0),
    rainShafts: readNumber(inputs.cloudRainShafts, 0),
    baseVariation: readNumber(inputs.cloudBaseVariation, 0.07),
    baseLumpiness: readNumber(inputs.cloudBaseLumpiness, 0.6),
    altocumulus: readNumber(inputs.cloudAltocumulus, 0),
    altostratus: readNumber(inputs.cloudAltostratus, 0),
    altoHeightMetres: readNumber(inputs.cloudAltoHeight, 4200),
    altoSpeed: readNumber(inputs.cloudAltoSpeed, 1)
  });
}

// A quick look at the mid-level layer. While the weather drives the
// clouds, it would put its own amounts back on the next frame, so the
// preset on show takes them too.
function showMidLevelClouds(altocumulus, altostratus, name) {
  inputs.cloudAltocumulus.value = String(altocumulus);
  inputs.cloudAltostratus.value = String(altostratus);

  for (const element of [inputs.cloudAltocumulus, inputs.cloudAltostratus]) {
    // Updates the slider's readout.
    element.dispatchEvent(new Event("input"));
  }

  if (inputs.cloudStyle.value === "off") {
    inputs.cloudStyle.value = "volumetric";
  }

  applyClouds();

  if (inputs.weatherEnabled.checked && inputs.weatherEffectClouds.checked) {
    const preset = inputs.weatherState.value;
    presetOverrides[preset] = { ...presetOverrides[preset], altocumulus, altostratus };
    applyWeather();
  }

  setStatus(`${name}: altocumulus ${altocumulus}, altostratus ${altostratus}.`);
}

function applyMist() {
  engine?.setMist({
    style: inputs.mistStyle.value,
    density: readNumber(inputs.mistDensity, 0.5),
    baseHeightMetres: readNumber(inputs.mistBaseHeight, 40),
    heightFalloffMetres: readNumber(inputs.mistHeightFalloff, 120),
    colour: [0.82, 0.85, 0.88],
    riseAboveWater: inputs.mistRiseAboveWater.checked,
    seedOffset: 5303,
    windSpeedMetresPerSecond: readNumber(inputs.mistWindSpeed, 2.5),
    windDirectionDegrees: readNumber(inputs.mistWindDirection, 70),
    sunScattering: readNumber(inputs.mistSunScattering, 0.6)
  });
}

function applyWeather() {
  engine?.setWeather({
    enabled: inputs.weatherEnabled.checked,
    state: inputs.weatherState.value,
    autoCycle: inputs.weatherAutoCycle.checked,
    allowSnow: inputs.weatherAllowSnow.checked,
    transitionSeconds: readNumber(inputs.weatherTransition, 20),
    windScale: readNumber(inputs.weatherWindScale, 1),
    precipitationScale: readNumber(inputs.weatherPrecipitation, 1),
    lensDrops: inputs.weatherLensDrops.checked,
    lensDropCount: readNumber(inputs.weatherLensDropCount, 60),
    lensDropMinSize: readNumber(inputs.weatherLensDropMin, 0.8) / 100,
    lensDropMaxSize: readNumber(inputs.weatherLensDropMax, 5) / 100,
    stateDurationSeconds: readNumber(inputs.weatherDuration, 240),
    effects: {
      clouds: inputs.weatherEffectClouds.checked,
      mist: inputs.weatherEffectMist.checked,
      wind: inputs.weatherEffectWind.checked,
      water: inputs.weatherEffectWater.checked,
      precipitation: inputs.weatherEffectPrecipitation.checked,
      ground: inputs.weatherEffectGround.checked,
      lightning: inputs.weatherEffectLightning.checked
    },
    presets: presetOverrides,
    regional: inputs.weatherRegional.checked
  });

  if (!inputs.weatherEnabled.checked && weatherReadout) {
    weatherReadout.textContent = "Weather: off";
  }

  syncWeatherChips();
}

// List every resolved preset, built in and custom, in the preset select.
function listWeatherPresets() {
  if (!engine) {
    return;
  }

  const selected = inputs.weatherState.value;
  const names = Object.keys(engine.getWeatherPresets());
  inputs.weatherState.replaceChildren(
    ...names.map((name) => {
      const option = document.createElement("option");
      option.value = name;
      option.textContent = weatherName(name).replace(/^./, (first) => first.toUpperCase());
      return option;
    })
  );
  inputs.weatherState.value = names.includes(selected) ? selected : "partlyCloudy";
}

// Show the selected preset's values on the editor's sliders.
let loadingPreset = false;

function loadPresetEditor() {
  const preset = engine?.getWeatherPresets()[inputs.weatherState.value];

  if (!preset) {
    return;
  }

  loadingPreset = true;

  for (const [id, field] of PRESET_FIELDS) {
    inputs[id].value = String(preset[field]);
    // Updates the slider's readout.
    inputs[id].dispatchEvent(new Event("input"));
  }

  loadingPreset = false;
}

// Write the editor's sliders into an override of the selected preset.
function editPreset() {
  if (loadingPreset) {
    return;
  }

  const override = {};

  for (const [id, field] of PRESET_FIELDS) {
    override[field] = readNumber(inputs[id], 0);
  }

  // Keeps any mid-level amounts a quick-look button put in the preset.
  presetOverrides[inputs.weatherState.value] = { ...presetOverrides[inputs.weatherState.value], ...override };
  applyWeather();
}

function resetPreset() {
  delete presetOverrides[inputs.weatherState.value];
  applyWeather();
  loadPresetEditor();
}

function applyTimeOfDay() {
  engine?.setTimeOfDay({
    enabled: inputs.timeOfDayEnabled.checked,
    hours: readNumber(inputs.timeOfDayHours, 12),
    dayLengthMinutes: readNumber(inputs.timeOfDayLength, 24),
    latitudeDegrees: readNumber(inputs.timeOfDayLatitude, 45)
  });
  showTimeOfDay();
}

function formatHours(hours) {
  if (hours === null) {
    return "none";
  }

  const minutes = Math.round(hours * 60) % 1440;
  return `${String(Math.floor(minutes / 60)).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
}

// While the time of day runs, the hour slider follows it.
function showTimeOfDay() {
  if (!engine || !timeOfDayReadout) {
    return;
  }

  const time = engine.getTimeOfDay();

  if (inputs.timeOfDayEnabled.checked && document.activeElement !== inputs.timeOfDayHours) {
    inputs.timeOfDayHours.value = time.hours.toFixed(1);
    inputs.timeOfDayHours.dispatchEvent(new Event("input"));
  }

  timeOfDayReadout.textContent =
    `${formatHours(time.hours)}, sun ${time.sunElevationDegrees.toFixed(1)}° high; ` +
    `sunrise ${formatHours(time.sunriseHours)}, sunset ${formatHours(time.sunsetHours)}`;
}

function wireWeatherPresets() {
  listWeatherPresets();
  loadPresetEditor();
  inputs.weatherState.addEventListener("change", loadPresetEditor);

  for (const [id] of PRESET_FIELDS) {
    inputs[id].addEventListener("input", editPreset);
  }

  buttons.presetReset.addEventListener("click", resetPreset);
  buttons.weatherSkip.addEventListener("click", () => {
    try {
      engine?.advanceWeather(600);
      setStatus("Skipped the weather ahead 10 minutes.");
    } catch (error) {
      showError(error);
    }
  });
  inputs.weatherRegional.addEventListener("input", applyWeather);

  for (const control of [inputs.timeOfDayEnabled, inputs.timeOfDayLength, inputs.timeOfDayLatitude]) {
    control.addEventListener("input", applyTimeOfDay);
  }

  // Dragging the hour sets the time; the running clock moves the slider
  // without calling back into the engine.
  inputs.timeOfDayHours.addEventListener("input", (event) => {
    if (event.isTrusted) {
      applyTimeOfDay();
    }
  });
  showTimeOfDay();
}

function applyShadows() {
  const strength = readNumber(inputs.shadowStrength, 0.85);
  engine?.setShadows({
    terrain: {
      enabled: inputs.terrainShadows.checked,
      strength,
      softness: readNumber(inputs.terrainShadowSoftness, 0.35)
    },
    trees: {
      enabled: inputs.treeShadows.checked,
      strength,
      distanceMetres: readNumber(inputs.treeShadowDistance, 260),
      resolution: readNumber(inputs.treeShadowResolution, 2048),
      softness: readNumber(inputs.treeShadowSoftness, 0.5)
    },
    clouds: {
      enabled: inputs.cloudShadowsEnabled.checked,
      strength: readNumber(inputs.cloudShadowStrength, 0.78)
    }
  });
}

function applySurface() {
  engine?.setSurface({
    textures: inputs.surfaceTextures.checked,
    detailNormals: inputs.surfaceNormals.checked,
    textureScale: readNumber(inputs.surfaceTextureScale, 1),
    rockiness: readNumber(inputs.surfaceRockiness, 1),
    boulders: inputs.surfaceBoulders.checked,
    boulderDistanceMetres: readNumber(inputs.surfaceBoulderDistance, 300)
  });
}

async function replaceTextureFromFile() {
  const file = inputs.replaceFile.files?.[0];

  if (!engine || !file) {
    return;
  }

  await checkImage(file, "a texture image");
  const [target, layer] = inputs.replaceTarget.value.split(":");
  const rgba = await imageToRgba(file);
  engine.replaceTexture(target, Number(layer), rgba);
  setStatus(`Replaced the ${inputs.replaceTarget.selectedOptions[0].textContent} texture.`);
}

// A stylised fir built in code, to show that any mesh can replace a
// species: a bark trunk and four stacked needle cones, in metres with the
// trunk base at the origin.
function buildFirModel() {
  const positions = [];
  const normals = [];
  const uvs = [];
  const indices = [];
  const textureLayers = [];
  const sides = 9;

  function ring(y, radius, v, layer, slope) {
    const start = positions.length / 3;

    for (let i = 0; i <= sides; i += 1) {
      const angle = (i / sides) * Math.PI * 2;
      const x = Math.cos(angle);
      const z = Math.sin(angle);
      const length = Math.hypot(1, slope);
      positions.push(x * radius, y, z * radius);
      normals.push(x / length, slope / length, z / length);
      // Tile the texture around and up the cone rather than stretching one
      // copy over the whole surface.
      uvs.push((i / sides) * 4, v * 3);
      textureLayers.push(layer);
    }

    return start;
  }

  function join(lower, upper) {
    for (let i = 0; i < sides; i += 1) {
      indices.push(lower + i, upper + i, lower + i + 1, lower + i + 1, upper + i, upper + i + 1);
    }
  }

  // Trunk: layer 1 is pine bark.
  join(ring(0, 0.35, 0, 1, 0), ring(9, 0.12, 3, 1, 0));

  // Needle cones: layer 6 is conifer needles (alpha-tested foliage).
  const tiers = [
    [2.5, 3.2, 7.5],
    [5, 2.5, 9.5],
    [7.5, 1.8, 11.5],
    [10, 1.1, 13.5]
  ];

  for (const [bottom, radius, top] of tiers) {
    const slope = radius / (top - bottom);
    join(ring(bottom, radius, 0, 6, slope), ring(top, 0.05, 1, 6, slope));
  }

  return { positions, normals, uvs, indices, textureLayers };
}

function toggleCustomModel() {
  if (!engine) {
    return;
  }

  customModelActive = !customModelActive;

  if (customModelActive) {
    engine.setTreeModel("spruce", buildFirModel());
    buttons.customModel.textContent = "Restore the procedural spruce";
    setStatus("Spruce trees now use a custom model, including their impostors and shadows.");
  } else {
    engine.resetTreeModel("spruce");
    buttons.customModel.textContent = "Replace spruce with a custom model";
    setStatus("Restored the procedural spruce.");
  }
}

function terrainHeightAt(x, z) {
  if (!latestMetadata || !latestHeights) {
    return null;
  }

  const { width, height, metresPerSample } = latestMetadata;
  const column = Math.round(x / metresPerSample + (width - 1) / 2);
  const row = Math.round(z / metresPerSample + (height - 1) / 2);

  if (column < 0 || row < 0 || column >= width || row >= height) {
    return null;
  }

  const view = new DataView(latestHeights.buffer, latestHeights.byteOffset, latestHeights.byteLength);
  return view.getFloat32((row * width + column) * 4, true);
}

function toggleGrove() {
  if (!engine) {
    return;
  }

  if (groveActive) {
    engine.setTreeInstances(undefined);
    groveActive = false;
    buttons.plantGrove.textContent = "Plant a hand-placed grove here";
    setStatus("Restored procedural tree placement.");
    return;
  }

  plantGrove(lastCamera?.target ?? [0, 0, 0]);
}

function plantGrove(centre) {
  const species = ["oak", "pine", "spruce", "acacia", "shrub"];
  const ground = inputs.groundGrove.checked;
  const trees = [];

  // Three rings of trees around the point the camera is looking at.
  for (let ringIndex = 1; ringIndex <= 3; ringIndex += 1) {
    const count = ringIndex * 8;

    for (let i = 0; i < count; i += 1) {
      const angle = (i / count) * Math.PI * 2 + ringIndex;
      const x = centre[0] + Math.cos(angle) * ringIndex * 14;
      const z = centre[2] + Math.sin(angle) * ringIndex * 14;
      const y = terrainHeightAt(x, z);

      if (y === null || y <= readNumber(inputs.sea, 0)) {
        continue;
      }

      trees.push({
        x,
        y,
        z,
        species: species[(i + ringIndex) % species.length],
        scale: 0.8 + ((i * 7) % 5) * 0.1,
        rotation: angle,
        ground
      });
    }
  }

  if (trees.length === 0) {
    setStatus("Look at dry land to plant a grove.");
    return;
  }

  engine.setTreeInstances(trees);
  groveActive = true;
  groveCentre = centre;
  buttons.plantGrove.textContent = "Restore procedural trees";
  const how = ground ? "grounded on the drawn terrain" : "at their given heights";
  setStatus(
    `Planted ${trees.length} hand-placed trees ${how}; procedural trees are hidden until restored.`
  );
}

// Show each slider's current value beside its label.
function initialiseSliderReadouts() {
  for (const slider of document.querySelectorAll('.controls input[type="range"]')) {
    const label = slider.closest("label");

    if (!label) {
      continue;
    }

    const readout = document.createElement("span");
    const step = slider.step && slider.step !== "any" ? slider.step : "1";
    const decimals = step.includes(".") ? step.split(".")[1].length : 0;
    readout.className = "value";
    readout.setAttribute("aria-hidden", "true");
    label.insertBefore(readout, slider);

    // A marked value names itself, such as "1.00 (old maximum)".
    const update = () => {
      const value = Number(slider.value);
      const mark = slider.dataset.mark !== undefined && value === Number(slider.dataset.mark);
      readout.textContent = value.toFixed(decimals) + (mark ? ` (${slider.dataset.markLabel})` : "");
    };

    slider.addEventListener("input", update);
    update();
  }
}

const SECTION_STORAGE_KEY = "vista-demo-open-sections";
// Sections added after open states were first saved as a plain list.
const NEWER_SECTIONS = /^(view|paint-.*)$/;

// Remember which sections are open and closed. Browser storage can be
// unavailable (private windows, blocked storage), so failures keep the
// defaults, as does a section the saved state does not name.
function initialiseSections() {
  const sections = document.querySelectorAll(".controls details.section");
  let saved = null;

  try {
    saved = JSON.parse(localStorage.getItem(SECTION_STORAGE_KEY) ?? "null");
  } catch {
    saved = null;
  }

  if (Array.isArray(saved)) {
    const names = [...sections].map((section) => section.dataset.section);
    saved = { open: saved, closed: names.filter((name) => !saved.includes(name) && !NEWER_SECTIONS.test(name)) };
  }

  for (const section of sections) {
    const name = section.dataset.section;

    if (Array.isArray(saved?.open) && Array.isArray(saved?.closed) && (saved.open.includes(name) || saved.closed.includes(name))) {
      section.open = saved.open.includes(name);
    }

    section.addEventListener("toggle", () => {
      const state = { open: [], closed: [] };

      for (const item of sections) {
        state[item.open ? "open" : "closed"].push(item.dataset.section);
      }

      try {
        localStorage.setItem(SECTION_STORAGE_KEY, JSON.stringify(state));
      } catch {
        // Storage is optional; the sections still work without it.
      }
    });
  }
}

const OVERLAY_STORAGE_KEY = "vista-demo-overlays";
const overlays = {
  stats: { toggle: input("showStats"), element: statsPanel },
  minimap: { toggle: input("showMinimap"), element: minimap },
  hint: { toggle: input("showHint"), element: hint },
  status: { toggle: input("showStatus"), element: status },
  panel: { toggle: input("showPanel"), element: panel }
};

// Hidden overlays get the `hidden` attribute, so screen readers skip them too.
function showOverlay(name, shown) {
  const overlay = overlays[name];
  overlay.toggle.checked = shown;
  overlay.element.hidden = !shown;

  if (name === "panel") {
    shell.classList.toggle("panel-hidden", !shown);
    showControlsButton.hidden = shown;
  }
}

function setOverlay(name, shown) {
  // Keep keyboard focus somewhere visible when its panel goes.
  const refocus = name === "panel" && !shown && panel.contains(document.activeElement);
  showOverlay(name, shown);

  if (refocus) {
    showControlsButton.focus();
  }

  try {
    localStorage.setItem(
      OVERLAY_STORAGE_KEY,
      JSON.stringify(Object.fromEntries(Object.entries(overlays).map(([key, overlay]) => [key, overlay.toggle.checked])))
    );
  } catch {
    // Storage is optional; the toggles still work without it.
  }
}

function initialiseOverlays() {
  let saved = null;

  try {
    saved = JSON.parse(localStorage.getItem(OVERLAY_STORAGE_KEY) ?? "null");
  } catch {
    saved = null;
  }

  for (const [name, overlay] of Object.entries(overlays)) {
    // Phones start with the panel collapsed, so the view has the screen.
    const fallback = name !== "panel" || window.innerWidth >= 700;
    showOverlay(name, typeof saved?.[name] === "boolean" ? saved[name] : fallback);
    overlay.toggle.addEventListener("change", () => setOverlay(name, overlay.toggle.checked));
  }

  showControlsButton.addEventListener("click", () => {
    setOverlay("panel", true);
    overlays.panel.toggle.focus();
  });
  window.addEventListener("keydown", (event) => {
    const name = overlayForKey(event);

    if (name) {
      setOverlay(name, !overlays[name].toggle.checked);
    } else if (activeTab === "paint") {
      paintTab?.keyDown(event);
    }
  });
  window.addEventListener("keyup", (event) => {
    if (activeTab === "paint") {
      paintTab?.keyUp(event);
    }
  });
}

const tabs = { explore: button("tabExplore"), paint: button("tabPaint") };
const viewer = document.querySelector("#viewer");
const paintWorkspace = document.querySelector("#paintWorkspace");
let activeTab = "explore";
// Where the camera goes back to in Explore: where it was, or a framing of a rendered painting.
let explorePose = null;

function attachControls(pose) {
  const attached = attachFlyCameraControls(engine, canvas, {
    initialPosition: pose.position,
    initialYawDegrees: 180,
    initialPitchDegrees: -12,
    fieldOfViewDegrees: pose.fieldOfViewDegrees,
    moveSpeedMetresPerSecond: 120,
    onCameraChange: (camera) => {
      lastCamera = camera;

      if (!minimap.hidden) {
        const yaw = Math.atan2(
          camera.target[0] - camera.position[0],
          camera.target[2] - camera.position[2]
        );
        drawMinimapMarker(camera.position, yaw + Math.PI);
      }

      if (biomeReadout) {
        const biome = engine?.biomeAt(camera.position[0], camera.position[2]);
        const celsius = engine?.temperatureAt(camera.position[0], camera.position[2]) ?? null;
        const temperature = celsius === null ? "" : `, ${celsius.toFixed(1)} °C`;
        biomeReadout.textContent = `Biome: ${biome ?? "–"}${temperature}`;
      }
    }
  });

  if (pose.target) {
    attached.lookAt(pose.target);
  }

  return attached;
}

// Paint stops the 3D loop, so the GPU is idle, and detaches the camera,
// so the paint keys never move it. The Explore controls stay in the DOM,
// hidden, so their state survives.
function selectTab(name) {
  if (name === activeTab || !engine) {
    return;
  }

  activeTab = name;
  const painting = name === "paint";

  for (const [key, tab] of Object.entries(tabs)) {
    tab.setAttribute("aria-selected", String(key === name));
    tab.tabIndex = key === name ? 0 : -1;
  }

  for (const section of document.querySelectorAll(".controls details.section")) {
    section.hidden = section.classList.contains("paint-section") !== painting;
  }

  viewer.hidden = painting;
  paintWorkspace.hidden = !painting;

  if (painting) {
    engine.stop();
    const camera = controls.getCamera();
    explorePose = { position: camera.position, target: camera.target, fieldOfViewDegrees: controls.getFieldOfView() };
    controls.dispose();
    paintTab.activate();
  } else {
    paintTab.deactivate();
    controls = attachControls(explorePose);
    engine.start();
  }
}

function wireTabs() {
  const order = Object.keys(tabs);

  for (const [name, tab] of Object.entries(tabs)) {
    tab.addEventListener("click", () => selectTab(name));
    // Arrow keys, Home and End move between tabs, as the ARIA tabs pattern expects.
    tab.addEventListener("keydown", (event) => {
      const index = order.indexOf(name);
      const next = {
        ArrowRight: order[(index + 1) % order.length],
        ArrowLeft: order[(index + order.length - 1) % order.length],
        Home: order[0],
        End: order[order.length - 1]
      }[event.key];

      if (next) {
        event.preventDefault();
        selectTab(next);
        tabs[next].focus();
      }
    });
  }
}

/** After the Paint tab loads terrain: update what Explore knows of it, and frame it. */
async function paintedTerrainLoaded({ handle, doc, camera, show }) {
  latestMetadata = handle.metadata;

  if (groveActive) {
    engine.setTreeInstances(undefined);
    groveActive = false;
    buttons.plantGrove.textContent = "Plant a hand-placed grove here";
  }

  // The painting's sea level is the water's.
  if (readNumber(inputs.sea, 0) !== doc.seaLevelMetres) {
    inputs.sea.value = String(doc.seaLevelMetres);
    inputs.sea.dispatchEvent(new Event("input"));
  }

  await refreshExportData();

  if (camera) {
    explorePose = { ...camera, fieldOfViewDegrees: explorePose?.fieldOfViewDegrees };
  }

  if (show) {
    selectTab("explore");
  }

  setStatus(`Painted terrain loaded (${handle.metadata.width} x ${handle.metadata.height}).`);
}

function syncWeatherChips() {
  for (const chip of document.querySelectorAll(".chip[data-weather]")) {
    const active = inputs.weatherEnabled.checked && chip.dataset.weather === inputs.weatherState.value;
    chip.setAttribute("aria-pressed", String(active));
  }
}

function wireWeatherChips() {
  for (const chip of document.querySelectorAll(".chip[data-weather]")) {
    chip.addEventListener("click", () => {
      const active = chip.getAttribute("aria-pressed") === "true";
      inputs.weatherEnabled.checked = !active;
      inputs.weatherState.value = chip.dataset.weather;
      applyWeather();
    });
  }

  syncWeatherChips();
}

// "From preset" and "Default" leave a value unset, so the engine chooses it.
function optionalDistance(element) {
  return element.value === "" ? undefined : Number(element.value);
}

function applyQuality() {
  engine?.setRenderQuality({
    preset: inputs.quality.value,
    maxClipmapLevels: 7,
    floraDensityScale: 1,
    renderDistanceMetres: optionalDistance(inputs.renderDistance),
    vegetationDetailMetres: optionalDistance(inputs.vegetationDetail),
    canopyDistanceMetres: optionalDistance(inputs.canopyDistance),
    maxTreeTriangles: optionalDistance(inputs.maxTreeTriangles),
    grassDetailMetres: optionalDistance(inputs.grassDetail),
    splitTreeTiming: inputs.splitTreeTiming.checked,
    renderFadeMetres: optionalDistance(inputs.renderFade),
    detailDistanceMetres: optionalDistance(inputs.detailDistance),
    cloudDistanceMetres: optionalDistance(inputs.cloudDistance),
    cloudFadeMetres: optionalDistance(inputs.cloudFade),
    maxFrameRate: readNumber(inputs.maxFrameRate, 60),
    renderScale: readNumber(inputs.renderScale, 1),
    dynamicResolution: inputs.dynamicResolution.checked,
    minRenderScale: Math.min(
      readNumber(inputs.minRenderScale, 0.5),
      readNumber(inputs.renderScale, 1)
    )
  });
}

function applyDebugView() {
  engine?.setDebugView(inputs.debugView.value);

  if (biomeLegend) {
    biomeLegend.hidden = inputs.debugView.value !== "biomes";
  }
}

function applyAllLiveControls() {
  applySun();
  applyAtmosphere();
  applyWater();
  applyBiomes();
  applyFlora();
  applyGrass();
  applyClouds();
  applyMist();
  applyWeather();
  applyShadows();
  applySurface();
  applyQuality();
  applyDebugView();
}

async function refreshExportData() {
  if (!engine || !latestMetadata) {
    return;
  }

  latestHeights = engine.exportHeightmap();
  renderHeightmapToCanvas(minimapEnsured(), latestMetadata, latestHeights);
  const context = minimapEnsured().getContext("2d");
  minimapBaseImage = context?.getImageData(0, 0, latestMetadata.width, latestMetadata.height) ?? null;
}

function minimapEnsured() {
  if (!minimap) {
    throw new Error("Missing #minimap canvas.");
  }

  return minimap;
}

function drawMinimapMarker(position, yawRadians) {
  if (!minimapBaseImage || !latestMetadata) {
    return;
  }

  const canvasElement = minimapEnsured();
  const context = canvasElement.getContext("2d");

  if (!context) {
    return;
  }

  context.putImageData(minimapBaseImage, 0, 0);

  const { width, height, metresPerSample } = latestMetadata;
  const halfWidth = ((width - 1) * metresPerSample) / 2;
  const halfHeight = ((height - 1) * metresPerSample) / 2;
  const pixelX = (position[0] + halfWidth) / metresPerSample;
  const pixelY = (position[2] + halfHeight) / metresPerSample;

  context.save();
  context.translate(pixelX, pixelY);
  context.rotate(yawRadians);
  context.fillStyle = "#ff5c5c";
  context.strokeStyle = "#ffffff";
  context.lineWidth = Math.max(1, width / 220);

  const markerSize = Math.max(4, width / 60);
  context.beginPath();
  context.moveTo(0, -markerSize);
  context.lineTo(markerSize * 0.6, markerSize * 0.6);
  context.lineTo(-markerSize * 0.6, markerSize * 0.6);
  context.closePath();
  context.fill();
  context.stroke();
  context.restore();
}

async function generate() {
  if (!engine) {
    return;
  }

  setStatus("Generating terrain...");
  loading.show("Generating terrain", `Seed ${inputs.seed.value}`);
  buttons.generate.disabled = true;
  // Time each generation phase from its first progress event to the next
  // phase's, and show them as they run.
  const phases = [];
  const started = performance.now();
  let total = 0;
  const stopProgress = engine.on("progress", ({ phase, progress }) => {
    const now = performance.now();
    const last = phases[phases.length - 1];

    if (phase === "fractal") {
      return;
    }

    loading.progress(phase, progress);

    if (!last || last.phase !== phase) {
      if (last) {
        last.ms = now - last.started;
      }

      phases.push({ phase, started: now, ms: 0 });
    }

    setStatus(`Generating terrain: ${phase} ${Math.round(progress * 100)} %`);
  });

  // A preset's inflow belongs to its own map, so generated maps go back
  // to finding their own; one the user placed is kept, moved onto this
  // map if it was placed on a larger one.
  if (presetInflow) {
    inputs.riverInflow.value = "auto";
    inputs.riverInflowDischarge.disabled = true;
    presetInflow = false;
  }

  const half = ((readNumber(inputs.size, 512) - 1) * readNumber(inputs.horizontalScale, 12)) / 2;
  inflowAt = inflowAt.map((value) => Math.max(-half, Math.min(half, value)));

  try {
    const options = buildFractalOptions();
    const handle = await engine.generateFractal(options).finally(() => {
      total = performance.now() - started;
      const last = phases[phases.length - 1];

      if (last) {
        last.ms = performance.now() - last.started;
      }

      stopProgress?.();
    });
    const timings = phases.map((entry) => `${entry.phase} ${Math.round(entry.ms)} ms`).join(", ");
    latestMetadata = handle.metadata;

    // Hand-placed trees belong to the old terrain.
    if (groveActive) {
      engine.setTreeInstances(undefined);
      groveActive = false;
      buttons.plantGrove.textContent = "Plant a hand-placed grove here";
    }

    applyAllLiveControls();
    await refreshExportData();
    keepCameraOnMap(controls);
    keepCameraAboveGround(controls);
    loading.hide();
    setStatus(
      `Seed ${options.seed} ready (${handle.metadata.width}x${handle.metadata.height}) in ` +
        `${Math.round(total)} ms: ${timings}.`
    );
  } catch (error) {
    showError(error);
  } finally {
    buttons.generate.disabled = false;
  }
}

function wireLiveControls() {
  for (const element of [inputs.sun, inputs.sunElevation, inputs.sunIntensity]) {
    element.addEventListener("input", applySun);
  }

  for (const element of [inputs.haze, inputs.exposure]) {
    element.addEventListener("input", applyAtmosphere);
  }

  for (const element of [
    inputs.waterEnabled,
    inputs.sea,
    inputs.waveScale,
    inputs.reflectivity,
    inputs.wavesEnabled,
    inputs.waveAmplitude,
    inputs.waveLength,
    inputs.waveDirection,
    inputs.waveSteepness,
    inputs.waveSpeed,
    inputs.waveSpread,
    inputs.currentSpeed,
    inputs.currentDirection,
    inputs.waterClarity,
    inputs.waterFoam,
    inputs.waterEddies,
    inputs.waterRefraction
  ]) {
    element.addEventListener("input", applyWater);
  }

  // River changes re-carve the terrain, so apply them once the slider is
  // released rather than on every intermediate value.
  for (const element of [
    inputs.riversEnabled,
    inputs.riverCatchment,
    inputs.riverWidth,
    inputs.riverCurrent,
    inputs.riverSnowmelt,
    inputs.riverMeanders,
    inputs.riverMaturity,
    inputs.riverBraiding,
    inputs.riverSprings,
    inputs.riverWaterfalls,
    inputs.riverInflowDischarge,
    inputs.riverRiparian,
    inputs.waterReflections
  ]) {
    element.addEventListener("change", applyWater);
  }

  // Biome changes re-bake the whole surface; apply on release too.
  for (const element of [
    inputs.biomesEnabled,
    inputs.biomeTemperature,
    inputs.biomeMoisture,
    inputs.biomeScale,
    inputs.biomeVolcanism,
    inputs.biomeBeachHeight,
    inputs.biomeSnowLineAuto,
    inputs.biomeSnowLine,
    inputs.biomeCelsiusAuto,
    inputs.biomeCelsius
  ]) {
    element.addEventListener("change", applyBiomes);
  }

  for (const element of [
    inputs.floraEnabled,
    inputs.floraDensity,
    inputs.treeLine,
    inputs.speciesVariation,
    inputs.windStrength,
    inputs.meshDistance,
    inputs.treeVariants
  ]) {
    element.addEventListener("input", applyFlora);
  }

  inputs.treeQuality.addEventListener("change", applyFlora);
  inputs.palmBeaches.addEventListener("change", applyFlora);
  inputs.groundGrove.addEventListener("change", () => {
    if (engine && groveActive && groveCentre) {
      plantGrove(groveCentre);
    }
  });

  for (const element of [inputs.grassEnabled, inputs.grassDensity, inputs.grassViewDistance]) {
    element.addEventListener("input", applyGrass);
  }

  inputs.grassForestFloor.addEventListener("change", applyGrass);

  inputs.grassStyle.addEventListener("change", applyGrass);

  for (const element of [
    inputs.cloudCoverage,
    inputs.cloudSpeed,
    inputs.cloudHeight,
    inputs.cloudWindDirection,
    inputs.cloudEvolution,
    inputs.cloudThickness,
    inputs.cloudDensity,
    inputs.cloudShadows,
    inputs.cloudTemporal,
    inputs.cloudCirrus,
    inputs.cloudCirrusHeight,
    inputs.cloudCirrusSpeed,
    inputs.cloudResolution,
    inputs.cloudStratiform,
    inputs.cloudTowering,
    inputs.cloudBaseDarkness,
    inputs.cloudRaggedBase,
    inputs.cloudRainShafts,
    inputs.cloudBaseVariation,
    inputs.cloudBaseLumpiness,
    inputs.cloudAltocumulus,
    inputs.cloudAltostratus,
    inputs.cloudAltoHeight,
    inputs.cloudAltoSpeed
  ]) {
    element.addEventListener("input", applyClouds);
  }

  buttons.mackerelSky.addEventListener("click", () => showMidLevelClouds(0.7, 0, "Mackerel sky"));
  buttons.veiledSun.addEventListener("click", () => showMidLevelClouds(0, 0.7, "Veiled sun"));

  inputs.cloudStyle.addEventListener("change", applyClouds);
  inputs.cloudSteps.addEventListener("change", applyClouds);

  for (const element of [
    inputs.mistDensity,
    inputs.mistBaseHeight,
    inputs.mistHeightFalloff,
    inputs.mistRiseAboveWater,
    inputs.mistWindSpeed,
    inputs.mistWindDirection,
    inputs.mistSunScattering
  ]) {
    element.addEventListener("input", applyMist);
  }

  inputs.mistStyle.addEventListener("change", applyMist);

  for (const element of [
    inputs.weatherEnabled,
    inputs.weatherAutoCycle,
    inputs.weatherAllowSnow,
    inputs.weatherTransition,
    inputs.weatherWindScale,
    inputs.weatherPrecipitation,
    inputs.weatherLensDrops,
    inputs.weatherDuration,
    inputs.weatherEffectClouds,
    inputs.weatherEffectMist,
    inputs.weatherEffectWind,
    inputs.weatherEffectWater,
    inputs.weatherEffectPrecipitation,
    inputs.weatherEffectGround,
    inputs.weatherEffectLightning
  ]) {
    element.addEventListener("input", applyWeather);
  }

  inputs.weatherState.addEventListener("change", applyWeather);

  // The drop controls only mean something with drops on, and the smallest
  // size never passes the largest: dragging one past the other moves both.
  const lensDropControls = [
    inputs.weatherLensDropCount,
    inputs.weatherLensDropMin,
    inputs.weatherLensDropMax
  ];
  const syncLensDropControls = () => {
    for (const control of lensDropControls) {
      control.disabled = !inputs.weatherLensDrops.checked;
    }
  };
  const keepOrdered = (moved, other, larger) => {
    const value = Number(moved.value);

    if (larger ? value < Number(other.value) : value > Number(other.value)) {
      other.value = moved.value;
      other.dispatchEvent(new Event("input"));
    }
  };

  inputs.weatherLensDrops.addEventListener("input", syncLensDropControls);
  inputs.weatherLensDropMin.addEventListener("input", () =>
    keepOrdered(inputs.weatherLensDropMin, inputs.weatherLensDropMax, false)
  );
  inputs.weatherLensDropMax.addEventListener("input", () =>
    keepOrdered(inputs.weatherLensDropMax, inputs.weatherLensDropMin, true)
  );

  for (const control of lensDropControls) {
    control.addEventListener("input", applyWeather);
  }

  syncLensDropControls();

  for (const element of [
    inputs.terrainShadows,
    inputs.terrainShadowSoftness,
    inputs.treeShadows,
    inputs.treeShadowDistance,
    inputs.treeShadowSoftness,
    inputs.shadowStrength,
    inputs.cloudShadowsEnabled,
    inputs.cloudShadowStrength
  ]) {
    element.addEventListener("input", applyShadows);
  }

  inputs.treeShadowResolution.addEventListener("change", applyShadows);

  for (const element of [
    inputs.surfaceTextures,
    inputs.surfaceNormals,
    inputs.surfaceTextureScale
  ]) {
    element.addEventListener("input", applySurface);
  }

  // Rockiness re-bakes the ground and what grows on it: apply on release.
  for (const element of [
    inputs.surfaceRockiness,
    inputs.surfaceBoulders,
    inputs.surfaceBoulderDistance
  ]) {
    element.addEventListener("change", applySurface);
  }

  inputs.replaceFile.addEventListener("change", () => {
    replaceTextureFromFile().catch(showError);
  });
  buttons.customModel.addEventListener("click", () => {
    try {
      toggleCustomModel();
    } catch (error) {
      showError(error);
    }
  });
  buttons.plantGrove.addEventListener("click", () => {
    try {
      toggleGrove();
    } catch (error) {
      showError(error);
    }
  });
  buttons.resetTextures.addEventListener("click", () => {
    engine?.resetTextures();
    inputs.replaceFile.value = "";
    setStatus("Restored the procedural textures.");
  });

  inputs.quality.addEventListener("change", applyQuality);

  for (const element of [
    inputs.renderDistance,
    inputs.vegetationDetail,
    inputs.canopyDistance,
    inputs.maxTreeTriangles,
    inputs.grassDetail,
    inputs.splitTreeTiming,
    inputs.renderFade,
    inputs.detailDistance,
    inputs.cloudDistance,
    inputs.cloudFade,
    inputs.maxFrameRate,
    inputs.renderScale,
    inputs.dynamicResolution,
    inputs.minRenderScale
  ]) {
    element.addEventListener("change", applyQuality);
  }
  inputs.debugView.addEventListener("change", applyDebugView);
}

function wireExportButtons() {
  buttons.screenshot.addEventListener("click", () => {
    engine
      ?.exportSnapshot()
      .then((blob) => downloadBlob(blob, "vistawasm-screenshot.png"))
      .catch(showError);
  });

  buttons.downloadMap.addEventListener("click", async () => {
    if (!latestMetadata || !latestHeights) {
      return;
    }

    try {
      const blob = await exportHeightmapImage(latestMetadata, latestHeights);
      downloadBlob(blob, "vistawasm-terrain-map.png");
    } catch (error) {
      showError(error);
    }
  });

  buttons.downloadModel.addEventListener("click", () => {
    if (!latestMetadata || !latestHeights) {
      return;
    }

    try {
      const obj = exportTerrainObj(latestMetadata, latestHeights, { maxSamplesPerSide: 256 });
      downloadText(obj, "vistawasm-terrain.obj", "model/obj");
    } catch (error) {
      showError(error);
    }
  });

  buttons.downloadHeightmap.addEventListener("click", () => {
    if (!latestMetadata || !latestHeights) {
      return;
    }

    downloadRawHeightmap(latestMetadata, latestHeights);
  });
}

const exportReadout = document.querySelector("#exportReadout");

function describeBytes(bytes) {
  return bytes >= 1048576 ? `${(bytes / 1048576).toFixed(1)} MB` : `${Math.ceil(bytes / 1024)} KB`;
}

/** Run one export, reporting how long it took and what it wrote. */
async function runExport(label, work) {
  if (!engine) {
    return;
  }

  exportReadout.textContent = `Exporting ${label}...`;
  // Let the message paint before the export takes the main thread.
  await new Promise((resolve) => setTimeout(resolve, 0));
  const start = performance.now();

  try {
    const files = await work();
    const seconds = ((performance.now() - start) / 1000).toFixed(2);
    exportReadout.textContent = `${label} in ${seconds} s: ${files
      .map(([name, blob]) => `${name} (${describeBytes(blob.size)})`)
      .join(", ")}`;

    for (const [name, blob] of files) {
      downloadBlob(blob, name);
    }
  } catch (error) {
    exportReadout.textContent = `${label} failed.`;
    showError(error);
  }
}

function wireMapExport() {
  buttons.exportMap.addEventListener("click", () => {
    const kind = inputs.exportKind.value;
    const format = inputs.exportFormat.value;
    const side = Number(inputs.exportSize.value);
    const options = Number.isFinite(side) ? { size: [side, side] } : {};

    runExport(`the ${kind} map`, async () => {
      const map = engine.exportMap(kind, options);
      const name = `vistawasm-${kind}-${map.width}x${map.height}`;

      if (format === "png8" || format === "png16") {
        // 16-bit is for single-channel maps; the others stay 8-bit.
        const bitDepth = format === "png16" && map.channels === 1 && !map.encoding.legend ? 16 : 8;
        const blobs = await encodePng(map, { bitDepth });
        return Array.isArray(blobs)
          ? blobs.map((blob, index) => [`${name}-splat${index}.png`, blob])
          : [[`${name}.png`, blobs]];
      }

      const { bytes } = encodeRaw(map, { type: format });
      const extension = format === "uint16" ? "u16le.bin" : "f32le.bin";
      return [[`${name}x${map.channels}.${extension}`, new Blob([bytes])]];
    });
  });

  buttons.exportTrees.addEventListener("click", () => {
    runExport("the trees", async () => {
      const trees = engine.exportTrees();
      return [[`vistawasm-trees-${trees.length}.csv`, new Blob([treesToCsv(trees)], { type: "text/csv" })]];
    });
  });

  buttons.exportBundle.addEventListener("click", () => {
    const side = Number(inputs.exportSize.value);

    runExport("the bundle", async () => [[
      "vistawasm-bundle.zip",
      await exportBundle(engine, Number.isFinite(side) ? { size: [side, side] } : {})
    ]]);
  });
}

const importReadout = document.querySelector("#importReadout");

/**
 * Run one import, reporting how long it took. Warnings the engine raises
 * meanwhile (resampled maps, unmatched colours, painted ocean above the
 * sea) are shown in the status line.
 */
async function runImport(label, work) {
  if (!engine) {
    return;
  }

  importReadout.textContent = `Importing ${label}...`;
  loading.show(`Importing ${label}`);
  const warnings = [];
  const stopWarnings = engine.on("warning", ({ message }) => warnings.push(message));
  const stopProgress = engine.on("progress", ({ phase, progress }) => loading.progress(phase, progress));
  const start = performance.now();

  try {
    const loaded = await work((message) => warnings.push(message));

    if (loaded) {
      // A new terrain: hand-placed trees belong to the old one.
      latestMetadata = loaded.metadata;

      if (groveActive) {
        engine.setTreeInstances(undefined);
        groveActive = false;
        buttons.plantGrove.textContent = "Plant a hand-placed grove here";
      }

      await refreshExportData();
    }

    loading.hide();
    importReadout.textContent = `${label} in ${((performance.now() - start) / 1000).toFixed(2)} s.`;
    setStatus(warnings.length ? `Imported ${label}. Warning: ${warnings.join(" ")}` : `Imported ${label}.`);
  } catch (error) {
    importReadout.textContent = `${label} failed.`;
    showError(error);
  } finally {
    stopWarnings();
    stopProgress();
  }
}

function wireImport() {
  const file = (element) => element.files?.[0];

  // A 16-bit PNG from the Export section carries its height range.
  inputs.importHeight.addEventListener("change", async () => {
    const chosen = file(inputs.importHeight);
    buttons.loadHeightImage.disabled = !chosen;

    if (!chosen) {
      return;
    }

    try {
      await checkImage(chosen, "a heightmap image", MAX_TERRAIN_SIDE);
    } catch (error) {
      buttons.loadHeightImage.disabled = true;
      importReadout.textContent = error.message;
      return;
    }

    try {
      const range = JSON.parse((await decodePng(new Uint8Array(await chosen.arrayBuffer()))).text["vistawasm:range"] ?? "null");

      if (Array.isArray(range)) {
        [inputs.importMin.value, inputs.importMax.value] = range.map(String);
        importReadout.textContent = `Height range ${range[0]} m to ${range[1]} m, from the PNG.`;
      }
    } catch {
      // Not a PNG, or no range: the fields keep what was typed.
    }
  });

  buttons.loadHeightImage.addEventListener("click", () => {
    const chosen = file(inputs.importHeight);

    runImport("the heightmap image", async () => engine.loadHeightmapImage(await checkImage(chosen, "a heightmap image", MAX_TERRAIN_SIDE), {
      metresPerSample: Number(inputs.importMetres.value),
      minHeightMetres: Number(inputs.importMin.value),
      maxHeightMetres: Number(inputs.importMax.value)
    }));
  });

  const painted = [
    [inputs.importBiome, "a biome map", async (image, onWarning) => {
      engine.setBiomeMap((await biomeMapFromImage(image, { onWarning })).map);
    }],
    [inputs.importWater, "a water mask", async (image, onWarning) => {
      engine.setWaterMask(await waterMaskFromImage(image, { onWarning }));
    }],
    [inputs.importTrees, "a tree mask", async (image, onWarning) => {
      engine.setVegetationMasks({ trees: await densityMaskFromImage(image, { onWarning }) });
    }],
    [inputs.importGrass, "a grass mask", async (image, onWarning) => {
      engine.setVegetationMasks({ grass: await densityMaskFromImage(image, { onWarning }) });
    }]
  ];

  for (const [element, label, apply] of painted) {
    element.addEventListener("change", () => {
      const chosen = file(element);

      if (chosen) {
        runImport(label.replace(/^a /, "the "), async (onWarning) => apply(await checkImage(chosen, label, MAX_TERRAIN_SIDE), onWarning));
      }

      element.value = "";
    });
  }

  buttons.clearPainted.addEventListener("click", () => {
    runImport("no painted maps", async () => {
      engine.setBiomeMap(null);
      engine.setWaterMask(null);
      engine.setVegetationMasks({ trees: null, grass: null });
    });
  });

  inputs.importBundle.addEventListener("change", () => {
    const chosen = file(inputs.importBundle);

    if (chosen) {
      runImport("the bundle", () => loadBundle(engine, checkFile(chosen, "zip", "a bundle")));
    }

    inputs.importBundle.value = "";
  });
}

async function run() {
  if (!canvas) {
    throw new Error("Missing canvas.");
  }

  const rect = canvas.getBoundingClientRect();
  engine = await createVistaEngine(canvas, {
    render: {
      width: Math.max(1, Math.floor(rect.width)),
      height: Math.max(1, Math.floor(rect.height)),
      devicePixelRatio: window.devicePixelRatio
    }
  });

  controls = attachControls({ position: [0, 420, 900] });

  buttons.jumpToWaterfall.addEventListener("click", () => jumpToWaterfall(controls));
  buttons.treeShowcase.addEventListener("click", () => showTrees(controls));
  buttons.jumpToMainRiver.addEventListener("click", () => jumpToMainRiver(controls));
  // A continental map with open edges, 15 km across, where a big river
  // flows in from beyond the map.
  buttons.riverValley.addEventListener("click", async () => {
    inputs.landform.value = "continental";
    inputs.edges.value = "open";
    inputs.size.value = "512";
    inputs.horizontalScale.value = "30";
    inputs.horizontalScale.dispatchEvent(new Event("input"));
    inputs.riverInflow.value = "auto";
    inputs.riverInflowDischarge.disabled = true;
    await generate();
    jumpToMainRiver(controls);
  });
  // A broad lowland valley where a big river meanders over its
  // floodplain. Generated maps carry no main river flatter than 0.5 %,
  // which meanders need, so this builds the floodplain itself.
  const loadMeanderingLowland = async () => {
    const settings = [
      [inputs.riverMeanders, "0.9"],
      [inputs.riverMaturity, "0.8"],
      [inputs.riverInflowDischarge, "60"]
    ];

    for (const [slider, value] of settings) {
      slider.value = value;
      slider.dispatchEvent(new Event("input"));
    }

    inputs.riverInflow.value = "camera";
    inputs.riverInflowDischarge.disabled = false;
    // The valley's head, 165 m in from the map's western edge, on its axis.
    inflowAt = [-7500, 15];
    presetInflow = true;
    setStatus("Building a meandering lowland...");
    loading.show("Building a meandering lowland", "A river 60 m³/s strong");
    const stopProgress = engine.on("progress", ({ phase, progress }) => loading.progress(phase, progress));
    const handle = await engine.loadRawHeightmap(lowlandHeights().buffer, {
      width: LOWLAND_SIZE,
      height: LOWLAND_SIZE,
      sampleFormat: "float32",
      metresPerSample: 30,
      heightScaleMetres: 1,
      seaLevelMetres: 0
    }).finally(() => {
      stopProgress?.();
      loading.hide();
    });
    latestMetadata = handle.metadata;
    applyAllLiveControls();
    await refreshExportData();
  };
  buttons.meanderingLowland.addEventListener("click", async () => {
    try {
      await loadMeanderingLowland();
      jumpToMainRiver(controls);
    } catch (error) {
      showError(error);
    }
  });
  // Two more shaped valleys: generated maps rarely braid or build deltas,
  // since their trunks are steep.
  const shapedPreset = async (heights, metres, status, settings, inflow) => {
    for (const [slider, value] of settings) {
      slider.value = value;
      slider.dispatchEvent(new Event("input"));
    }

    inputs.riverInflow.value = "camera";
    inputs.riverInflowDischarge.disabled = false;
    inflowAt = inflow;
    presetInflow = true;
    setStatus(status);

    try {
      const handle = await engine.loadRawHeightmap(heights.buffer, {
        width: LOWLAND_SIZE,
        height: LOWLAND_SIZE,
        sampleFormat: "float32",
        metresPerSample: metres,
        heightScaleMetres: 1,
        seaLevelMetres: 0
      });
      latestMetadata = handle.metadata;
      applyAllLiveControls();
      await refreshExportData();
      jumpToMainRiver(controls);
    } catch (error) {
      showError(error);
    }
  };
  buttons.braidedValley.addEventListener("click", () =>
    shapedPreset(
      braidedHeights(),
      BRAIDED_METRES,
      "Building a braided valley...",
      [
        [inputs.riverBraiding, "1"],
        [inputs.riverMeanders, "0.3"],
        [inputs.riverInflowDischarge, "60"]
      ],
      // The plain's head, 165 m in from the western edge, on its axis.
      [-4945, 10]
    )
  );
  buttons.deltaCoast.addEventListener("click", () =>
    shapedPreset(
      deltaHeights(),
      30,
      "Building a delta coast...",
      [
        [inputs.riverMeanders, "0.6"],
        [inputs.riverMaturity, "0.5"],
        [inputs.riverInflowDischarge, "400"]
      ],
      [-7500, 15]
    )
  );
  inputs.riverInflow.addEventListener("change", () => {
    presetInflow = false;
    inputs.riverInflowDischarge.disabled = inputs.riverInflow.value !== "camera";

    if (inputs.riverInflow.value === "camera") {
      placeInflowAtCamera(controls);
    }

    applyWater();
  });

  const observer = new ResizeObserver(() => {
    if (!engine) {
      return;
    }

    const next = canvas.getBoundingClientRect();

    // A hidden canvas (the Paint tab) keeps its size until it is shown again.
    if (next.width === 0 || next.height === 0) {
      return;
    }

    engine.resize(
      Math.max(1, Math.floor(next.width)),
      Math.max(1, Math.floor(next.height)),
      window.devicePixelRatio
    );
  });

  observer.observe(canvas);

  // The render loop stops itself on these; say so rather than freezing silently.
  engine.on("deviceLost", () => {
    setStatus("The GPU device was lost, so rendering stopped. Reload the page to start again.");
  });
  engine.on("fatalError", showError);

  // Time between frames is what the viewer sees. `frameTimeMs` only counts
  // the CPU time to submit a frame, not the GPU time to draw it.
  let lastFrameAt = performance.now();
  let smoothedFrameMs = 1000 / 60;

  engine.on("stats", (stats) => {
    if (inputs.timeOfDayEnabled.checked) {
      showTimeOfDay();
    }

    const now = performance.now();
    smoothedFrameMs += (now - lastFrameAt - smoothedFrameMs) * 0.1;
    lastFrameAt = now;

    // Hidden stats skip building their text every frame.
    if (!statsPanel || statsPanel.hidden) {
      return;
    }

    const fps = Math.round(1000 / Math.max(1, smoothedFrameMs));
    statsPanel.textContent = [
      `FPS ${fps} (${smoothedFrameMs.toFixed(1)} ms per frame)`,
      `Render resolution ${Math.round((stats.renderScale ?? 1) * 100)} %`,
      `Triangles ${stats.terrainTriangles.toLocaleString()}`,
      `Trees drawn ${stats.floraInstances.toLocaleString()}`,
      `Grass drawn ${stats.grassInstances.toLocaleString()}`,
      `Clipmap levels ${stats.clipmapLevels}`,
      `Weather ${stats.weather ? weatherName(stats.weather) : "off"}`,
      ...localWeatherLines(),
      `Water sounds: ${lastCamera ? describeWaterSounds(engine.getWaterSounds(...lastCamera.position)) : "–"}`,
      ...gpuProfile(stats)
    ].join("\n");
  });

  engine.on("weatherChanged", (kind) => {
    if (weatherReadout) {
      weatherReadout.textContent = kind ? `Weather: ${weatherName(kind)}` : "Weather: off";
    }
  });

  window.addEventListener("beforeunload", () => {
    controls?.dispose();
    observer.disconnect();
    engine?.stop();
    engine?.dispose();
  });

  paintTab = createPaintTab({
    engine,
    getSun: () => [readNumber(inputs.sun, 132), readNumber(inputs.sunElevation, 30)],
    onTerrain: (loaded) => {
      paintedTerrainLoaded(loaded).catch(showError);
    }
  });
  initialiseSections();
  initialiseOverlays();
  wireTabs();
  initialiseSliderReadouts();
  wireLiveControls();
  wireWeatherChips();
  wireWeatherPresets();
  wireExportButtons();
  wireMapExport();
  wireImport();
  // Iteration sliders only apply when custom counts are chosen; otherwise
  // the quality preset sets them.
  const syncErosionControls = () => {
    inputs.hydraulicIterations.disabled = !inputs.erosionCustom.checked;
    inputs.thermalIterations.disabled = !inputs.erosionCustom.checked;
  };
  inputs.erosionCustom.addEventListener("change", syncErosionControls);
  syncErosionControls();

  buttons.generate.addEventListener("click", () => {
    generate().catch(showError);
  });

  engine.start();

  // Open on a river, seen from the water's edge: the meandering lowland's.
  // The terrain controls generate maps from there.
  try {
    await loadMeanderingLowland();

    if (viewRiver(controls)) {
      setStatus("Lowland river ready: 60 m³/s, seen from its bank. Generate terrain or try the presets.");
    } else {
      jumpToMainRiver(controls);
      setStatus("Lowland river ready. Generate terrain or try the presets.");
    }
  } catch (error) {
    showError(error);
    await generate();
  }
}

run().catch(showError);
