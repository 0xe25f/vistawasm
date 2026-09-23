<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import {
    VistaWasmError,
    attachFlyCameraControls,
    createVistaEngine,
    downloadBlob,
    downloadRawHeightmap,
    downloadText,
    exportHeightmapImage,
    exportTerrainObj,
    renderHeightmapToCanvas,
    type CloudStyle,
    type DebugView,
    type ErosionOptions,
    type FractalTerrainOptions,
    type GrassStyle,
    type MistStyle,
    type NoiseKind,
    type RenderQualityPreset,
    type TerrainMetadata,
    type TreeQuality,
    type VistaEngine
  } from "@vista-wasm/vista-wasm";

  let canvas: HTMLCanvasElement;
  let minimap: HTMLCanvasElement;
  let panel: HTMLElement;
  let status = $state("Starting VistaWASM...");
  let stats = $state("");
  let generating = $state(false);

  let engine: VistaEngine | null = null;
  let flyControls: { dispose(): void } | null = null;
  let observer: ResizeObserver | null = null;
  let latestMetadata: TerrainMetadata | null = null;
  let latestHeights: Uint8Array<ArrayBuffer> | null = null;
  let minimapBaseImage: ImageData | null = null;

  function input(id: string): HTMLInputElement {
    const element = panel.querySelector<HTMLInputElement>(`#${id}`);

    if (!element) {
      throw new Error(`Missing #${id} control.`);
    }

    return element;
  }

  function select(id: string): HTMLSelectElement {
    const element = panel.querySelector<HTMLSelectElement>(`#${id}`);

    if (!element) {
      throw new Error(`Missing #${id} control.`);
    }

    return element;
  }

  function readNumber(element: { value: string }, fallback: number): number {
    const value = Number(element.value);
    return Number.isFinite(value) ? value : fallback;
  }

  function showError(error: unknown): void {
    status =
      error instanceof VistaWasmError
        ? `${error.code}: ${error.message}`
        : error instanceof Error
          ? error.message
          : "VistaWASM failed.";
  }

  function buildFractalOptions(): FractalTerrainOptions {
    const erosion: ErosionOptions | undefined = input("erosionEnabled").checked
      ? {
          hydraulicIterations: readNumber(input("hydraulicIterations"), 0),
          thermalIterations: readNumber(input("thermalIterations"), 0),
          quality: "preview"
        }
      : undefined;

    return {
      seed: readNumber(input("seed"), 3333),
      size: readNumber(select("size"), 512) as FractalTerrainOptions["size"],
      horizontalScaleMetres: readNumber(input("horizontalScale"), 10),
      verticalScale: readNumber(input("verticalScale"), 1),
      seaLevelMetres: readNumber(input("sea"), 0),
      noise: {
        kind: select("noiseKind").value as NoiseKind,
        octaves: readNumber(input("octaves"), 7),
        gain: 0.5,
        lacunarity: 2,
        warp: readNumber(input("warp"), 0.1)
      },
      shape: {
        island: readNumber(input("shapeIsland"), 0),
        terrace: readNumber(input("shapeTerrace"), 0),
        basin: readNumber(input("shapeBasin"), 0),
        canyon: readNumber(input("shapeCanyon"), 0),
        crater: readNumber(input("shapeCrater"), 0)
      },
      erosion
    };
  }

  function applySun(): void {
    engine?.setSun({
      azimuthDegrees: readNumber(input("sun"), 132),
      elevationDegrees: readNumber(input("sunElevation"), 30),
      intensity: readNumber(input("sunIntensity"), 1.3)
    });
  }

  function applyAtmosphere(): void {
    engine?.setAtmosphere({
      rayleighStrength: 1,
      mieStrength: 0.45,
      hazeDistanceMetres: readNumber(input("haze"), 60000),
      exposure: readNumber(input("exposure"), 1.1),
      skyTint: [1, 1, 1]
    });
  }

  function applyWater(): void {
    engine?.setWater({
      enabled: input("waterEnabled").checked,
      seaLevelMetres: readNumber(input("sea"), 0),
      waveScale: readNumber(input("waveScale"), 0.8),
      reflectivity: readNumber(input("reflectivity"), 0.4),
      shorelineSoftnessMetres: 6
    });
  }

  function applyFlora(): void {
    engine?.setFlora({
      enabled: input("floraEnabled").checked,
      density: readNumber(input("floraDensity"), 0.35),
      treeLineMetres: readNumber(input("treeLine"), 1800),
      seedOffset: 3001,
      maxInstances: 20000,
      treeQuality: select("treeQuality").value as TreeQuality,
      speciesVariation: readNumber(input("speciesVariation"), 0.6),
      windStrength: readNumber(input("windStrength"), 0.3)
    });
  }

  function applyGrass(): void {
    engine?.setGrass({
      enabled: input("grassEnabled").checked,
      style: select("grassStyle").value as GrassStyle,
      density: readNumber(input("grassDensity"), 0.5),
      viewDistanceMetres: readNumber(input("grassViewDistance"), 220),
      seedOffset: 7331,
      maxInstances: 60000
    });
  }

  function applyClouds(): void {
    engine?.setClouds({
      style: select("cloudStyle").value as CloudStyle,
      coverage: readNumber(input("cloudCoverage"), 0.45),
      speed: readNumber(input("cloudSpeed"), 1),
      heightMetres: readNumber(input("cloudHeight"), 4000),
      colour: [1, 1, 1],
      seedOffset: 9007,
      raymarchSteps: 24
    });
  }

  function applyMist(): void {
    engine?.setMist({
      style: select("mistStyle").value as MistStyle,
      density: readNumber(input("mistDensity"), 0.5),
      baseHeightMetres: readNumber(input("mistBaseHeight"), 40),
      heightFalloffMetres: readNumber(input("mistHeightFalloff"), 120),
      colour: [0.82, 0.85, 0.88],
      riseAboveWater: input("mistRiseAboveWater").checked,
      seedOffset: 5303
    });
  }

  function applyQuality(): void {
    engine?.setRenderQuality({
      preset: select("quality").value as RenderQualityPreset,
      maxClipmapLevels: 7,
      floraDensityScale: 1
    });
  }

  function applyDebugView(): void {
    engine?.setDebugView(select("debugView").value as DebugView);
  }

  function applyAllLiveControls(): void {
    applySun();
    applyAtmosphere();
    applyWater();
    applyFlora();
    applyGrass();
    applyClouds();
    applyMist();
    applyQuality();
    applyDebugView();
  }

  function drawMinimapMarker(position: [number, number, number], yawRadians: number): void {
    if (!minimapBaseImage || !latestMetadata || !minimap) {
      return;
    }

    const context = minimap.getContext("2d");

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

  async function refreshExportData(): Promise<void> {
    if (!engine || !latestMetadata || !minimap) {
      return;
    }

    latestHeights = engine.exportHeightmap();
    renderHeightmapToCanvas(minimap, latestMetadata, latestHeights);
    const context = minimap.getContext("2d");
    minimapBaseImage = context?.getImageData(0, 0, latestMetadata.width, latestMetadata.height) ?? null;
  }

  async function generate(): Promise<void> {
    if (!engine) {
      return;
    }

    generating = true;
    status = "Generating terrain...";

    try {
      const options = buildFractalOptions();
      const handle = await engine.generateFractal(options);
      latestMetadata = handle.metadata;
      applyAllLiveControls();
      await refreshExportData();
      status = `Seed ${options.seed} ready (${handle.metadata.width}x${handle.metadata.height}).`;
    } catch (error) {
      showError(error);
    } finally {
      generating = false;
    }
  }

  function exportScreenshot(): void {
    engine
      ?.exportSnapshot()
      .then((blob) => downloadBlob(blob, "vistawasm-screenshot.png"))
      .catch(showError);
  }

  async function exportMap(): Promise<void> {
    if (!latestMetadata || !latestHeights) {
      return;
    }

    try {
      const blob = await exportHeightmapImage(latestMetadata, latestHeights);
      downloadBlob(blob, "vistawasm-terrain-map.png");
    } catch (error) {
      showError(error);
    }
  }

  function exportModel(): void {
    if (!latestMetadata || !latestHeights) {
      return;
    }

    try {
      const obj = exportTerrainObj(latestMetadata, latestHeights, { maxSamplesPerSide: 256 });
      downloadText(obj, "vistawasm-terrain.obj", "model/obj");
    } catch (error) {
      showError(error);
    }
  }

  function exportHeightmapBytes(): void {
    if (!latestMetadata || !latestHeights) {
      return;
    }

    downloadRawHeightmap(latestMetadata, latestHeights);
  }

  onMount(() => {
    async function run() {
      const rect = canvas.getBoundingClientRect();
      engine = await createVistaEngine(canvas, {
        render: {
          width: Math.max(1, Math.floor(rect.width)),
          height: Math.max(1, Math.floor(rect.height)),
          devicePixelRatio: window.devicePixelRatio
        }
      });

      flyControls = attachFlyCameraControls(engine, canvas, {
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
        }
      });

      observer = new ResizeObserver(() => {
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

      // Time between frames is what the viewer sees. `frameTimeMs` only counts
      // the CPU time to submit a frame, not the GPU time to draw it.
      let lastFrameAt = performance.now();
      let smoothedFrameMs = 1000 / 60;

      engine.on("stats", (frameStats) => {
        const now = performance.now();
        smoothedFrameMs += (now - lastFrameAt - smoothedFrameMs) * 0.1;
        lastFrameAt = now;
        const fps = Math.round(1000 / Math.max(1, smoothedFrameMs));
        stats = [
          `FPS ${fps} (${smoothedFrameMs.toFixed(1)} ms per frame)`,
          `Triangles ${frameStats.terrainTriangles.toLocaleString()}`,
          `Flora instances ${frameStats.floraInstances.toLocaleString()}`,
          `Grass instances ${frameStats.grassInstances.toLocaleString()}`,
          `Clipmap levels ${frameStats.clipmapLevels}`
        ].join("\n");
      });

      engine.start();
      await generate();
    }

    run().catch(showError);
  });

  onDestroy(() => {
    flyControls?.dispose();
    observer?.disconnect();
    engine?.stop();
    engine?.dispose();
    engine = null;
  });
</script>

<main class="shell">
  <section class="viewer">
    <canvas bind:this={canvas} class="vista-canvas" aria-label="VistaWASM terrain view"></canvas>
    <canvas
      bind:this={minimap}
      class="minimap"
      aria-label="Terrain minimap"
      title="Top-down terrain map"
    ></canvas>
    <div class="vista-status">{status}</div>
    <div class="stats">{stats}</div>
    <div class="hint">
      Drag to look &middot; WASD to move &middot; Space/Shift or middle-drag to rise and fall &middot; Scroll to zoom
    </div>
  </section>
  <aside bind:this={panel} class="controls" aria-label="VistaWASM controls">
    <fieldset>
      <legend>Terrain</legend>
      <label>
        Seed
        <input id="seed" type="number" value="3333" step="1" />
      </label>
      <label>
        Size
        <select id="size">
          <option value="256">256</option>
          <option value="512" selected>512</option>
          <option value="1024">1024</option>
          <option value="2048">2048</option>
        </select>
      </label>
      <label>
        Noise kind
        <select id="noiseKind">
          <option value="ridged" selected>Ridged</option>
          <option value="simplex">Simplex</option>
          <option value="hybrid">Hybrid</option>
          <option value="island">Island</option>
          <option value="canyon">Canyon</option>
          <option value="cratered">Cratered</option>
          <option value="classic">Classic</option>
        </select>
      </label>
      <label>
        Octaves
        <input id="octaves" type="range" min="1" max="12" value="7" />
      </label>
      <label>
        Horizontal scale (m)
        <input id="horizontalScale" type="range" min="2" max="40" value="10" />
      </label>
      <label>
        Vertical scale
        <input id="verticalScale" type="range" min="0.2" max="3" step="0.1" value="1" />
      </label>
      <label>
        Domain warp
        <input id="warp" type="range" min="0" max="2" step="0.05" value="0.1" />
      </label>
    </fieldset>

    <fieldset>
      <legend>Shape</legend>
      <label>
        Island falloff
        <input id="shapeIsland" type="range" min="0" max="1" step="0.05" value="0" />
      </label>
      <label>
        Terrace
        <input id="shapeTerrace" type="range" min="0" max="1" step="0.05" value="0" />
      </label>
      <label>
        Basin
        <input id="shapeBasin" type="range" min="0" max="1" step="0.05" value="0" />
      </label>
      <label>
        Canyon
        <input id="shapeCanyon" type="range" min="0" max="1" step="0.05" value="0" />
      </label>
      <label>
        Crater
        <input id="shapeCrater" type="range" min="0" max="1" step="0.05" value="0" />
      </label>
    </fieldset>

    <fieldset>
      <legend>Erosion</legend>
      <label class="checkbox">
        <input id="erosionEnabled" type="checkbox" checked />
        Enabled
      </label>
      <label>
        Hydraulic iterations
        <input id="hydraulicIterations" type="range" min="0" max="60" value="10" />
      </label>
      <label>
        Thermal iterations
        <input id="thermalIterations" type="range" min="0" max="60" value="8" />
      </label>
    </fieldset>

    <fieldset>
      <legend>Sky and light</legend>
      <label>
        Sun azimuth
        <input id="sun" type="range" min="0" max="360" value="132" />
      </label>
      <label>
        Sun elevation
        <input id="sunElevation" type="range" min="-10" max="85" value="30" />
      </label>
      <label>
        Sun intensity
        <input id="sunIntensity" type="range" min="0.2" max="3" step="0.1" value="1.3" />
      </label>
      <label>
        Haze distance (m)
        <input id="haze" type="range" min="1000" max="120000" value="60000" />
      </label>
      <label>
        Exposure
        <input id="exposure" type="range" min="0.4" max="2" step="0.05" value="1.1" />
      </label>
    </fieldset>

    <fieldset>
      <legend>Water</legend>
      <label class="checkbox">
        <input id="waterEnabled" type="checkbox" checked />
        Enabled
      </label>
      <label>
        Sea level (m)
        <input id="sea" type="range" min="-200" max="400" value="0" />
      </label>
      <label>
        Wave scale
        <input id="waveScale" type="range" min="0" max="3" step="0.1" value="0.8" />
      </label>
      <label>
        Reflectivity
        <input id="reflectivity" type="range" min="0" max="1" step="0.05" value="0.4" />
      </label>
    </fieldset>

    <fieldset>
      <legend>Vegetation</legend>
      <label class="checkbox">
        <input id="floraEnabled" type="checkbox" checked />
        Enabled
      </label>
      <label>
        Density
        <input id="floraDensity" type="range" min="0" max="1" step="0.05" value="0.35" />
      </label>
      <label>
        Tree line (m)
        <input id="treeLine" type="range" min="200" max="3000" value="1800" />
      </label>
      <label>
        Tree quality
        <select id="treeQuality">
          <option value="billboard">Billboard impostors (fastest)</option>
          <option value="cross-quad">Cross-quad</option>
          <option value="mesh" selected>3D meshes + impostors (realistic)</option>
        </select>
      </label>
      <label>
        Species variation
        <input id="speciesVariation" type="range" min="0" max="1" step="0.05" value="0.6" />
      </label>
      <label>
        Wind strength
        <input id="windStrength" type="range" min="0" max="1" step="0.05" value="0.3" />
      </label>
    </fieldset>

    <fieldset>
      <legend>Grass</legend>
      <label class="checkbox">
        <input id="grassEnabled" type="checkbox" />
        Enabled
      </label>
      <label>
        Style
        <select id="grassStyle">
          <option value="billboard-blades" selected>Billboard blades</option>
          <option value="dense-blades">Dense blades (hyper-realistic)</option>
        </select>
      </label>
      <label>
        Density
        <input id="grassDensity" type="range" min="0" max="1" step="0.05" value="0.5" />
      </label>
      <label>
        View distance (m)
        <input id="grassViewDistance" type="range" min="30" max="600" value="220" />
      </label>
    </fieldset>

    <fieldset>
      <legend>Clouds</legend>
      <label>
        Style
        <select id="cloudStyle">
          <option value="off" selected>Off</option>
          <option value="painted">Painted</option>
          <option value="volumetric">Volumetric (hyper-realistic)</option>
        </select>
      </label>
      <label>
        Coverage
        <input id="cloudCoverage" type="range" min="0" max="1" step="0.05" value="0.45" />
      </label>
      <label>
        Drift speed
        <input id="cloudSpeed" type="range" min="0" max="3" step="0.1" value="1" />
      </label>
      <label>
        Layer height (m)
        <input id="cloudHeight" type="range" min="500" max="12000" value="4000" />
      </label>
    </fieldset>

    <fieldset>
      <legend>Mist</legend>
      <label>
        Style
        <select id="mistStyle">
          <option value="off" selected>Off</option>
          <option value="flat">Flat</option>
          <option value="volumetric">Volumetric (hyper-realistic)</option>
        </select>
      </label>
      <label>
        Density
        <input id="mistDensity" type="range" min="0" max="1" step="0.05" value="0.5" />
      </label>
      <label>
        Base height (m)
        <input id="mistBaseHeight" type="range" min="-200" max="500" value="40" />
      </label>
      <label>
        Height falloff (m)
        <input id="mistHeightFalloff" type="range" min="10" max="500" value="120" />
      </label>
      <label class="checkbox">
        <input id="mistRiseAboveWater" type="checkbox" checked />
        Rise above water
      </label>
    </fieldset>

    <fieldset>
      <legend>Render</legend>
      <label>
        Quality preset
        <select id="quality">
          <option value="preview">Preview</option>
          <option value="balanced" selected>Balanced</option>
          <option value="high">High</option>
          <option value="offline">Offline</option>
        </select>
      </label>
      <label>
        Debug view
        <select id="debugView">
          <option value="none" selected>None</option>
          <option value="height">Height</option>
          <option value="slope">Slope</option>
          <option value="normals">Normals</option>
          <option value="lod">LOD</option>
          <option value="flow">Flow</option>
          <option value="materials">Materials</option>
          <option value="no-data">No data</option>
        </select>
      </label>
    </fieldset>

    <fieldset>
      <legend>Actions</legend>
      <button type="button" disabled={generating} onclick={() => generate().catch(showError)}>
        Generate terrain
      </button>
      <button type="button" onclick={exportScreenshot}>Save screenshot (PNG)</button>
      <button type="button" onclick={() => exportMap().catch(showError)}>Download terrain map (PNG)</button>
      <button type="button" onclick={exportModel}>Download 3D model (OBJ)</button>
      <button type="button" onclick={exportHeightmapBytes}>Download raw heightmap</button>
    </fieldset>
  </aside>
</main>
