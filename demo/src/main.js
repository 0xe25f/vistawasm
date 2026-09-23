import {
  VistaWasmError,
  attachFlyCameraControls,
  createVistaEngine,
  downloadBlob,
  downloadRawHeightmap,
  downloadText,
  exportHeightmapImage,
  exportTerrainObj,
  imageToRgba,
  renderHeightmapToCanvas
} from "@vista-wasm/vista-wasm";

const canvas = document.querySelector("#vista");
const minimap = document.querySelector("#minimap");
const status = document.querySelector("#status");
const statsPanel = document.querySelector("#stats");
const biomeReadout = document.querySelector("#biomeReadout");

const inputs = {
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
  riversEnabled: input("riversEnabled"),
  riverCatchment: input("riverCatchment"),
  riverWidth: input("riverWidth"),
  riverCurrent: input("riverCurrent"),
  biomesEnabled: input("biomesEnabled"),
  biomeTemperature: input("biomeTemperature"),
  biomeMoisture: input("biomeMoisture"),
  biomeScale: input("biomeScale"),
  biomeVolcanism: input("biomeVolcanism"),
  biomeBeachHeight: input("biomeBeachHeight"),
  biomeSnowLineAuto: input("biomeSnowLineAuto"),
  biomeSnowLine: input("biomeSnowLine"),
  floraEnabled: input("floraEnabled"),
  floraDensity: input("floraDensity"),
  treeLine: input("treeLine"),
  treeQuality: select("treeQuality"),
  speciesVariation: input("speciesVariation"),
  windStrength: input("windStrength"),
  meshDistance: input("meshDistance"),
  grassEnabled: input("grassEnabled"),
  grassStyle: select("grassStyle"),
  grassDensity: input("grassDensity"),
  grassViewDistance: input("grassViewDistance"),
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
  weatherDuration: input("weatherDuration"),
  weatherEffectClouds: input("weatherEffectClouds"),
  weatherEffectMist: input("weatherEffectMist"),
  weatherEffectWind: input("weatherEffectWind"),
  weatherEffectWater: input("weatherEffectWater"),
  weatherEffectPrecipitation: input("weatherEffectPrecipitation"),
  weatherEffectGround: input("weatherEffectGround"),
  weatherEffectLightning: input("weatherEffectLightning"),
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
  replaceTarget: select("replaceTarget"),
  replaceFile: input("replaceFile"),
  palmBeaches: input("palmBeaches"),
  quality: select("quality"),
  renderDistance: select("renderDistance"),
  detailDistance: select("detailDistance"),
  cloudDistance: select("cloudDistance"),
  debugView: select("debugView")
};

const buttons = {
  generate: button("generate"),
  screenshot: button("screenshot"),
  downloadMap: button("downloadMap"),
  downloadModel: button("downloadModel"),
  downloadHeightmap: button("downloadHeightmap"),
  resetTextures: button("resetTextures"),
  customModel: button("customModel"),
  plantGrove: button("plantGrove")
};

let customModelActive = false;
let groveActive = false;
let lastCamera = null;

const weatherReadout = document.querySelector("#weatherReadout");
const WEATHER_NAMES = {
  clear: "clear",
  partlyCloudy: "partly cloudy",
  overcast: "overcast",
  fog: "fog",
  rain: "rain",
  storm: "storm",
  snow: "snow"
};

let engine = null;
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

// Where each frame's GPU time goes, largest first, like a game's frame
// profiler.
function gpuProfile(stats) {
  const times = stats.gpuPassTimesMs;

  if (!times) {
    return ["GPU timing: not available in this browser"];
  }

  const names = {
    terrain: "Terrain",
    trees: "Trees",
    grass: "Grass",
    clouds: "Clouds",
    skyAndFog: "Sky and fog",
    water: "Water",
    shadows: "Shadows",
    treeCulling: "Tree culling",
    lens: "Lens drops"
  };
  const rows = Object.entries(names)
    .map(([key, name]) => [name, times[key]])
    .filter(([, ms]) => ms > 0.005)
    .sort((a, b) => b[1] - a[1])
    .map(([name, ms]) => `  ${name} ${ms.toFixed(2)} ms`);
  return [`GPU ${(stats.gpuFrameTimeMs ?? 0).toFixed(2)} ms per frame`, ...rows];
}

function setStatus(message) {
  if (status) {
    status.textContent = message;
  }
}

function showError(error) {
  if (error instanceof VistaWasmError) {
    setStatus(`${error.code}: ${error.message}`);
    return;
  }

  setStatus(error instanceof Error ? error.message : "VistaWASM failed.");
}

function buildFractalOptions() {
  const erosion = inputs.erosionEnabled.checked
    ? {
        hydraulicIterations: readNumber(inputs.hydraulicIterations, 0),
        thermalIterations: readNumber(inputs.thermalIterations, 0),
        quality: "preview"
      }
    : undefined;

  return {
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
      currentSpeed: readNumber(inputs.riverCurrent, 1)
    },
    currentSpeed: readNumber(inputs.currentSpeed, 0.35),
    currentDirectionDegrees: readNumber(inputs.currentDirection, 60),
    clarityMetres: readNumber(inputs.waterClarity, 6),
    foam: readNumber(inputs.waterFoam, 0.7)
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
      : readNumber(inputs.biomeSnowLine, 1400)
  });
}

function applyFlora() {
  engine?.setFlora({
    enabled: inputs.floraEnabled.checked,
    density: readNumber(inputs.floraDensity, 0.35),
    treeLineMetres: readNumber(inputs.treeLine, 1800),
    seedOffset: 3001,
    maxInstances: 20000,
    treeQuality: inputs.treeQuality.value,
    speciesVariation: readNumber(inputs.speciesVariation, 0.6),
    windStrength: readNumber(inputs.windStrength, 0.3),
    meshDistanceMetres: readNumber(inputs.meshDistance, 420),
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
    maxInstances: 60000
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
    rainShafts: readNumber(inputs.cloudRainShafts, 0)
  });
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
    stateDurationSeconds: readNumber(inputs.weatherDuration, 240),
    effects: {
      clouds: inputs.weatherEffectClouds.checked,
      mist: inputs.weatherEffectMist.checked,
      wind: inputs.weatherEffectWind.checked,
      water: inputs.weatherEffectWater.checked,
      precipitation: inputs.weatherEffectPrecipitation.checked,
      ground: inputs.weatherEffectGround.checked,
      lightning: inputs.weatherEffectLightning.checked
    }
  });

  if (!inputs.weatherEnabled.checked && weatherReadout) {
    weatherReadout.textContent = "Weather: off";
  }

  syncWeatherChips();
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
    textureScale: readNumber(inputs.surfaceTextureScale, 1)
  });
}

async function replaceTextureFromFile() {
  const file = inputs.replaceFile.files?.[0];

  if (!engine || !file) {
    return;
  }

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

  const centre = lastCamera?.target ?? [0, 0, 0];
  const species = ["oak", "pine", "spruce", "acacia", "shrub"];
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
        rotation: angle
      });
    }
  }

  if (trees.length === 0) {
    setStatus("Look at dry land to plant a grove.");
    return;
  }

  engine.setTreeInstances(trees);
  groveActive = true;
  buttons.plantGrove.textContent = "Restore procedural trees";
  setStatus(`Planted ${trees.length} hand-placed trees; procedural trees are hidden until restored.`);
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

    const update = () => {
      readout.textContent = Number(slider.value).toFixed(decimals);
    };

    slider.addEventListener("input", update);
    update();
  }
}

const SECTION_STORAGE_KEY = "vista-demo-open-sections";

// Remember which sections are open. Browser storage can be unavailable
// (private windows, blocked storage), so failures keep the defaults.
function initialiseSections() {
  const sections = document.querySelectorAll(".controls details.section");
  let saved = null;

  try {
    saved = JSON.parse(localStorage.getItem(SECTION_STORAGE_KEY) ?? "null");
  } catch {
    saved = null;
  }

  for (const section of sections) {
    if (Array.isArray(saved)) {
      section.open = saved.includes(section.dataset.section);
    }

    section.addEventListener("toggle", () => {
      const open = [...sections].filter((item) => item.open).map((item) => item.dataset.section);

      try {
        localStorage.setItem(SECTION_STORAGE_KEY, JSON.stringify(open));
      } catch {
        // Storage is optional; the sections still work without it.
      }
    });
  }
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

// "From preset" leaves a distance unset, so the preset chooses it.
function optionalDistance(element) {
  return element.value === "" ? undefined : Number(element.value);
}

function applyQuality() {
  engine?.setRenderQuality({
    preset: inputs.quality.value,
    maxClipmapLevels: 7,
    floraDensityScale: 1,
    renderDistanceMetres: optionalDistance(inputs.renderDistance),
    detailDistanceMetres: optionalDistance(inputs.detailDistance),
    cloudDistanceMetres: optionalDistance(inputs.cloudDistance)
  });
}

function applyDebugView() {
  engine?.setDebugView(inputs.debugView.value);
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
  buttons.generate.disabled = true;

  try {
    const options = buildFractalOptions();
    const handle = await engine.generateFractal(options);
    latestMetadata = handle.metadata;

    // Hand-placed trees belong to the old terrain.
    if (groveActive) {
      engine.setTreeInstances(undefined);
      groveActive = false;
      buttons.plantGrove.textContent = "Plant a hand-placed grove here";
    }

    applyAllLiveControls();
    await refreshExportData();
    setStatus(`Seed ${options.seed} ready (${handle.metadata.width}x${handle.metadata.height}).`);
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
    inputs.waterFoam
  ]) {
    element.addEventListener("input", applyWater);
  }

  // River changes re-carve the terrain, so apply them once the slider is
  // released rather than on every intermediate value.
  for (const element of [inputs.riversEnabled, inputs.riverCatchment, inputs.riverWidth, inputs.riverCurrent]) {
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
    inputs.biomeSnowLine
  ]) {
    element.addEventListener("change", applyBiomes);
  }

  for (const element of [
    inputs.floraEnabled,
    inputs.floraDensity,
    inputs.treeLine,
    inputs.speciesVariation,
    inputs.windStrength,
    inputs.meshDistance
  ]) {
    element.addEventListener("input", applyFlora);
  }

  inputs.treeQuality.addEventListener("change", applyFlora);
  inputs.palmBeaches.addEventListener("change", applyFlora);

  for (const element of [inputs.grassEnabled, inputs.grassDensity, inputs.grassViewDistance]) {
    element.addEventListener("input", applyGrass);
  }

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
    inputs.cloudRainShafts
  ]) {
    element.addEventListener("input", applyClouds);
  }

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

  for (const element of [inputs.renderDistance, inputs.detailDistance, inputs.cloudDistance]) {
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

  const controls = attachFlyCameraControls(engine, canvas, {
    initialPosition: [0, 420, 900],
    initialYawDegrees: 180,
    initialPitchDegrees: -12,
    moveSpeedMetresPerSecond: 120,
    onCameraChange: (camera) => {
      lastCamera = camera;
      const yaw = Math.atan2(
        camera.target[0] - camera.position[0],
        camera.target[2] - camera.position[2]
      );
      drawMinimapMarker(camera.position, yaw + Math.PI);

      if (biomeReadout) {
        const biome = engine?.biomeAt(camera.position[0], camera.position[2]);
        biomeReadout.textContent = `Biome: ${biome ?? "–"}`;
      }
    }
  });

  const observer = new ResizeObserver(() => {
    if (!engine) {
      return;
    }

    const next = canvas.getBoundingClientRect();
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
    if (!statsPanel) {
      return;
    }

    const now = performance.now();
    smoothedFrameMs += (now - lastFrameAt - smoothedFrameMs) * 0.1;
    lastFrameAt = now;
    const fps = Math.round(1000 / Math.max(1, smoothedFrameMs));
    statsPanel.textContent = [
      `FPS ${fps} (${smoothedFrameMs.toFixed(1)} ms per frame)`,
      `Triangles ${stats.terrainTriangles.toLocaleString()}`,
      `Flora instances ${stats.floraInstances.toLocaleString()}`,
      `Grass instances ${stats.grassInstances.toLocaleString()}`,
      `Clipmap levels ${stats.clipmapLevels}`,
      `Weather ${stats.weather ? WEATHER_NAMES[stats.weather] : "off"}`,
      ...gpuProfile(stats)
    ].join("\n");
  });

  engine.on("weatherChanged", (kind) => {
    if (weatherReadout) {
      weatherReadout.textContent = kind ? `Weather: ${WEATHER_NAMES[kind]}` : "Weather: off";
    }
  });

  window.addEventListener("beforeunload", () => {
    controls.dispose();
    observer.disconnect();
    engine?.stop();
    engine?.dispose();
  });

  initialiseSections();
  initialiseSliderReadouts();
  wireLiveControls();
  wireWeatherChips();
  wireExportButtons();
  buttons.generate.addEventListener("click", () => {
    generate().catch(showError);
  });

  engine.start();
  await generate();
}

run().catch(showError);
