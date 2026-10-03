// The budgeted scenes, shared by `fixed-scene.mjs` (software WebGPU in
// headless Chromium) and `bench/gpu/` (real GPUs), so the two never
// drift. Plain data and one helper, with no Node or DOM imports: it loads
// in Node and in the browser alike.
//
// A scene is `{ terrain, engine, calls, camera }`:
//
// - `terrain` is `{ heightmap: true }` for the fixed heightmap below, or
//   `generateFractal` options;
// - `engine` holds `createVistaEngine` options;
// - `calls` maps engine methods to their argument, called in order;
// - `camera` is a full camera, or `{ ground: { position, target } }` with
//   `[x, z, metres above the ground]` for each, placed on the heightmap.
//
// Scenes that share `terrain` and `engine` can share one engine; `group`
// names that set, and `id` names one scene without spaces or commas, for
// command lines and URL parameters.

// `fixed-512.f32.gz`: 512 x 512 float32 samples at 12 m, loaded with
// `loadRawHeightmap`, so generator changes do not change the scene.
export const heightmapFile = "fixed-512.f32.gz";

export const heightmapOptions = {
  width: 512,
  height: 512,
  sampleFormat: "float32",
  byteOrder: "little-endian",
  metresPerSample: 12,
  heightScaleMetres: 1,
  seaLevelMetres: 0
};

// The `generateFractal` options a generated scene starts from; a scene's
// `terrain` is merged over them (the visual-check page does the same).
export const fractalDefaults = {
  seed: 12345,
  size: 512,
  horizontalScaleMetres: 12,
  verticalScale: 1,
  seaLevelMetres: 0,
  noise: { kind: "ridged", octaves: 7, gain: 0.52, lacunarity: 2.05, warp: 0.15 },
  shape: { island: 0.35 }
};

/**
 * `heightAt(x, z)` for `applyCalls`: the nearest sample of `heights` (from
 * `exportHeightmap()`), or 0 off the map.
 */
export function heightSampler(heights, metadata) {
  return (x, z) => {
    const sx = Math.round(x / metadata.metresPerSample + (metadata.width - 1) / 2);
    const sz = Math.round(z / metadata.metresPerSample + (metadata.height - 1) / 2);

    if (sx < 0 || sz < 0 || sx >= metadata.width || sz >= metadata.height) {
      return 0;
    }

    return heights[sz * metadata.width + sx];
  };
}

/** A scene name with spaces as hyphens, as `fixed-scene.mjs` prints it. */
export function slug(name) {
  return name.replace(/,/g, "").replace(/ +/g, "-");
}

export const camera = {
  position: [-1500, 1500, 2600],
  target: [0, 300, 0],
  fieldOfViewDegrees: 55,
  nearMetres: 0.5,
  farMetres: 120000
};

export const grass = {
  enabled: true,
  style: "billboard-blades",
  density: 0.5,
  viewDistanceMetres: 220,
  seedOffset: 7331,
  maxInstances: 200000
};

const flora = {
  enabled: true,
  density: 4,
  treeLineMetres: 1800,
  seedOffset: 3001,
  maxInstances: 500000,
  treeQuality: "mesh",
  speciesVariation: 0.6,
  windStrength: 0.3,
  meshDistanceMetres: 420,
  speciesRules: []
};

// Dynamic resolution would change the work between runs.
export const quality = { preset: "balanced", dynamicResolution: false, renderScale: 1 };

/**
 * The fixed scene's engine options. `grass` pins the grass settings (it
 * is on by default), `noGrass` switches it off, `jungle` adds the dense
 * forest and `split` times the trees pass as three passes.
 */
export function fixedEngine({ grass: withGrass = false, noGrass = false, jungle = false, split = false } = {}) {
  return {
    quality: { ...quality, ...(split ? { splitTreeTiming: true } : {}) },
    ...(withGrass || jungle ? { grass } : {}),
    ...(noGrass ? { grass: { ...grass, enabled: false } } : {}),
    ...(jungle ? { flora } : {})
  };
}

// The fixed scene: clear, then rain with lens drops, then -18 °C looking
// out over pack ice. Each keeps what the one before it set.
export const fixedScenes = {
  clear: {},
  rain: {
    setWeather: {
      enabled: true,
      state: "rain",
      autoCycle: false,
      transitionSeconds: 0.1,
      lensDrops: true,
      lensDropCount: 120
    }
  },
  // Pack ice on the open sea, far enough past the map edge that nothing
  // but sea ice is on screen.
  ice: {
    setWeather: { enabled: false },
    setBiomes: { meanTemperatureCelsius: -18 },
    setCamera: { ...camera, position: [9000, 900, 400], target: [13000, 0, 1200] }
  }
};

// The jungle: the same heightmap, warm and wet (27 °C, moisture bias
// 0.7), with trees at density 4 and grass on. `ground` places a camera
// `[x, z, metres above the ground]` and its target likewise.
export const jungleScenes = {
  closeUp: {
    setBiomes: { meanTemperatureCelsius: 27, moistureBias: 0.7 },
    ground: { position: [40, 2040, 6], target: [0, 2000, 4] }
  },
  // The edge of the open foothills, looking into the stand.
  clearing: { ground: { position: [1180, 2300, 20], target: [880, 2260, 8] } },
  hillside: { ground: { position: [0, 2950, 300], target: [0, 1700, 0] } }
};

// The meadow: open, gently rolling grassland on `rollingHills` (seed
// 12345), with trees off so every band of distance is grass. The camera
// looks across it diagonally, away from the sun.
export const meadowTerrain = { landform: "rollingHills" };

export function meadowEngine(density) {
  return {
    quality,
    grass: { ...grass, density },
    flora: { ...flora, enabled: false, density: 1 }
  };
}

export function meadowCamera(height) {
  return { ground: { position: [-790, -90, height], target: [-640, 60, 0] } };
}

export const meadowDensities = [0.5, 4];
export const meadowHeights = [2, 4, 10, 25, 30, 60];

// The sky views, over the sea at (0, 40, -4000) south of the fixed
// heightmap, looking north towards it. The sun is at its default
// azimuth, 132 degrees.
const sea = [0, 40, -4000];
const lookFrom = (direction) => ({
  ...camera,
  position: sea,
  target: [sea[0] + direction[0], sea[1] + direction[1], sea[2] + direction[2]]
});
const fair = (preset = {}) => ({
  setWeather: {
    enabled: true,
    state: "partlyCloudy",
    autoCycle: false,
    transitionSeconds: 0.1,
    presets: { partlyCloudy: preset }
  }
});
const sunAt = (elevationDegrees) => ({ azimuthDegrees: 132, elevationDegrees, intensity: 1.2 });
// Towards the sun at `elevation` degrees, 1 km out.
const towardsSun = (elevation) => {
  const [a, e] = [(132 * Math.PI) / 180, (elevation * Math.PI) / 180];
  return [1000 * Math.cos(a) * Math.cos(e), 1000 * Math.sin(e), 1000 * Math.sin(a) * Math.cos(e)];
};

/** Altocumulus at `amount`, looking up at 50 degrees. */
export function mackerelSky(amount) {
  return {
    ...fair({ altocumulus: amount, altostratus: 0 }),
    setSun: sunAt(18),
    setCamera: lookFrom([0, 1192, 1000])
  };
}

export const skyScenes = {
  "cumulus from below": { ...fair(), setSun: sunAt(18), setCamera: lookFrom([400, 700, 2000]) },
  "low sun": { ...fair(), setSun: sunAt(12), setCamera: lookFrom([400, 700, 2000]) },
  "mackerel sky": mackerelSky(0.7),
  "veiled sun": {
    ...fair({ altocumulus: 0, altostratus: 0.7 }),
    setSun: sunAt(35),
    setCamera: lookFrom(towardsSun(35))
  },
  "rain deck": {
    setWeather: { enabled: true, state: "rain", autoCycle: false, transitionSeconds: 0.1 },
    setSun: sunAt(18),
    setCamera: lookFrom([400, 700, 2000])
  }
};

/**
 * Every budgeted scene, in the order `bench/gpu/` runs them, with the
 * budget (whole frame, or one pass) it is held to, in milliseconds on a
 * mid-range GPU at 1920 x 1080. `id` is the name as `slug` writes it:
 * `fixed-clear`, `fixed-clear-grass`, `jungle-closeUp`, `meadow-0.5-4-m`,
 * `sky-cumulus-from-below`.
 */
export const budgetedScenes = [
  ...Object.keys(fixedScenes).map((name) => ({
    name: `fixed ${name}`,
    group: "fixed",
    terrain: { heightmap: true },
    engine: fixedEngine(),
    calls: fixedScenes[name],
    budget: { frame: name === "rain" ? 14 : 12 }
  })),
  ...Object.keys(fixedScenes).map((name) => ({
    name: `fixed ${name}, grass`,
    group: "fixed-grass",
    terrain: { heightmap: true },
    engine: fixedEngine({ grass: true }),
    calls: fixedScenes[name],
    budget: { frame: name === "rain" ? 14 : 12 }
  })),
  ...Object.keys(jungleScenes).map((name) => ({
    name: `jungle ${name}`,
    group: "jungle",
    terrain: { heightmap: true },
    engine: fixedEngine({ jungle: true, split: true }),
    calls: jungleScenes[name],
    budget: name === "closeUp" ? { frame: 14 } : {}
  })),
  ...meadowDensities.flatMap((density) => [4, 25].map((height) => ({
    name: `meadow ${density}, ${height} m`,
    group: `meadow-${density}`,
    terrain: meadowTerrain,
    engine: meadowEngine(density),
    calls: meadowCamera(height),
    budget: height === 4 ? { grass: density === 0.5 ? 1 : 2 } : {}
  }))),
  ...Object.keys(skyScenes).map((name) => ({
    name: `sky ${name}`,
    group: "sky",
    terrain: { heightmap: true },
    engine: fixedEngine(),
    calls: skyScenes[name],
    budget: name === "rain deck" ? { frame: 14 } : { frame: 12 }
  }))
].map((scene) => ({ id: slug(scene.name), ...scene }));

/** The `group` of every budgeted scene, in order, each once. */
export const budgetedGroups = [...new Set(budgetedScenes.map((scene) => scene.group))];

/**
 * Apply a scene's `calls` to `engine`: each method with its argument, and
 * `ground` as a camera placed on the ground with `heightAt(x, z)`.
 */
export function applyCalls(engine, calls, heightAt) {
  for (const [method, value] of Object.entries(calls)) {
    if (method === "ground") {
      const [px, pz, up] = value.position;
      const [tx, tz, targetUp] = value.target;
      engine.setCamera({
        ...camera,
        position: [px, heightAt(px, pz) + up, pz],
        target: [tx, heightAt(tx, tz) + targetUp, tz]
      });
    } else {
      engine[method](value);
    }
  }
}
