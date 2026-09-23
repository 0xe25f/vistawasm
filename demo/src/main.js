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
  currentSpeed: input("currentSpeed"),
  currentDirection: input("currentDirection"),
  waterClarity: input("waterClarity"),
  waterFoam: input("waterFoam"),
  riversEnabled: input("riversEnabled"),
  riverCatchment: input("riverCatchment"),
  riverCurrent: input("riverCurrent"),
  biomesEnabled: input("biomesEnabled"),
  biomeTemperature: input("biomeTemperature"),
  biomeMoisture: input("biomeMoisture"),
  biomeScale: input("biomeScale"),
  biomeVolcanism: input("biomeVolcanism"),
  floraEnabled: input("floraEnabled"),
  floraDensity: input("floraDensity"),
  treeLine: input("treeLine"),
  treeQuality: select("treeQuality"),
  speciesVariation: input("speciesVariation"),
  windStrength: input("windStrength"),
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
  terrainShadows: input("terrainShadows"),
  terrainShadowSoftness: input("terrainShadowSoftness"),
  treeShadows: input("treeShadows"),
  treeShadowDistance: input("treeShadowDistance"),
  treeShadowResolution: select("treeShadowResolution"),
  shadowStrength: input("shadowStrength"),
  surfaceTextures: input("surfaceTextures"),
  surfaceNormals: input("surfaceNormals"),
  surfaceTextureScale: input("surfaceTextureScale"),
  replaceTarget: select("replaceTarget"),
  replaceFile: input("replaceFile"),
  quality: select("quality"),
  debugView: select("debugView")
};

const buttons = {
  generate: button("generate"),
  screenshot: button("screenshot"),
  downloadMap: button("downloadMap"),
  downloadModel: button("downloadModel"),
  downloadHeightmap: button("downloadHeightmap"),
  resetTextures: button("resetTextures")
};

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
      steepness: readNumber(inputs.waveSteepness, 0.55)
    },
    rivers: {
      enabled: inputs.riversEnabled.checked,
      minCatchmentKm2: readNumber(inputs.riverCatchment, 0.15),
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
    volcanism: readNumber(inputs.biomeVolcanism, 0.35)
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
    windStrength: readNumber(inputs.windStrength, 0.3)
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
    raymarchSteps: 32,
    windDirectionDegrees: readNumber(inputs.cloudWindDirection, 70),
    evolution: readNumber(inputs.cloudEvolution, 0.35),
    thicknessMetres: readNumber(inputs.cloudThickness, 1600),
    density: readNumber(inputs.cloudDensity, 0.6),
    castShadows: inputs.cloudShadows.checked
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
    precipitationScale: readNumber(inputs.weatherPrecipitation, 1)
  });

  if (!inputs.weatherEnabled.checked && weatherReadout) {
    weatherReadout.textContent = "Weather: off";
  }
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
      resolution: readNumber(inputs.treeShadowResolution, 2048)
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

function applyQuality() {
  engine?.setRenderQuality({
    preset: inputs.quality.value,
    maxClipmapLevels: 7,
    floraDensityScale: 1
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
    inputs.currentSpeed,
    inputs.currentDirection,
    inputs.waterClarity,
    inputs.waterFoam
  ]) {
    element.addEventListener("input", applyWater);
  }

  // River changes re-carve the terrain, so apply them once the slider is
  // released rather than on every intermediate value.
  for (const element of [inputs.riversEnabled, inputs.riverCatchment, inputs.riverCurrent]) {
    element.addEventListener("change", applyWater);
  }

  // Biome changes re-bake the whole surface; apply on release too.
  for (const element of [
    inputs.biomesEnabled,
    inputs.biomeTemperature,
    inputs.biomeMoisture,
    inputs.biomeScale,
    inputs.biomeVolcanism
  ]) {
    element.addEventListener("change", applyBiomes);
  }

  for (const element of [inputs.floraEnabled, inputs.floraDensity, inputs.treeLine, inputs.speciesVariation, inputs.windStrength]) {
    element.addEventListener("input", applyFlora);
  }

  inputs.treeQuality.addEventListener("change", applyFlora);

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
    inputs.cloudShadows
  ]) {
    element.addEventListener("input", applyClouds);
  }

  inputs.cloudStyle.addEventListener("change", applyClouds);

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
    inputs.weatherPrecipitation
  ]) {
    element.addEventListener("input", applyWeather);
  }

  inputs.weatherState.addEventListener("change", applyWeather);

  for (const element of [
    inputs.terrainShadows,
    inputs.terrainShadowSoftness,
    inputs.treeShadows,
    inputs.treeShadowDistance,
    inputs.shadowStrength
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
  buttons.resetTextures.addEventListener("click", () => {
    engine?.resetTextures();
    inputs.replaceFile.value = "";
    setStatus("Restored the procedural textures.");
  });

  inputs.quality.addEventListener("change", applyQuality);
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

  engine.on("stats", (stats) => {
    if (!statsPanel) {
      return;
    }

    const fps = stats.frameTimeMs > 0 ? Math.round(1000 / stats.frameTimeMs) : 0;
    statsPanel.textContent = [
      `FPS ~${fps}`,
      `Triangles ${stats.terrainTriangles.toLocaleString()}`,
      `Flora instances ${stats.floraInstances.toLocaleString()}`,
      `Grass instances ${stats.grassInstances.toLocaleString()}`,
      `Clipmap levels ${stats.clipmapLevels}`
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

  wireLiveControls();
  wireExportButtons();
  buttons.generate.addEventListener("click", () => {
    generate().catch(showError);
  });

  engine.start();
  await generate();
}

run().catch(showError);
