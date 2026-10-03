# Changelog

All notable changes to VistaWASM are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [2.0.0] — 2026-10-03

A realism release: geology-led terrain, biomes, real trees, simulated
water, weather, and shadows, with every system switchable, tunable, and
replaceable, plus map export and import, a painting demo, and a project
site. Every input is checked against documented limits, and the engine
recovers cleanly from failed loads and GPU errors. The public API only
grows, but saved seeds now make new maps and
some options changed meaning (grass `density`, erosion iteration
counts), so this is a major version. See
[Upgrading from 1.0.0](#upgrading-from-100) for what changed.

### Added

- **Biomes.** Nineteen climate-driven biomes: grassy meadows, outer
  thicket, outer and inner forest, mountain foothills, mountain proper,
  outer volcanic, caldera, savannah, coastal beach, coastal rocky, outer
  and inner jungle, swamp wetlands, ocean, alpine transition, lower and
  upper snowy peaks, and ice and arctic. New `BiomeOptions`,
  `setBiomes()`, `biomeAt(x, z)`, and a `"biomes"` debug view.
- **Procedural textures**, generated on the GPU at start-up with nothing
  to download: ten terrain materials with height, normal, occlusion, and
  roughness; bark, leaf, needle, frond, and moss textures; water ripples;
  2D and 3D noise.
- **Trees.** Eight procedurally modelled species (oak, pine, spruce, palm,
  jungle, cypress, acacia, shrub) with 3D meshes near the camera, baked
  impostors in the distance, GPU culling, and indirect draws.
- **Water.** Gerstner wave simulation (`WaterOptions.waves`), surface
  currents, depth-based colour and clarity, foam, and shoaling surf; rivers
  and lakes from the terrain's drainage network, carved into the terrain,
  with flowing currents (`WaterOptions.rivers`).
- **Weather.** Clear, partly cloudy, overcast, fog, rain, storm, and snow,
  with smooth transitions, optional automatic cycling, and per-effect
  control. It drives clouds, mist, wind, waves, falling rain and snow, wet
  ground and puddles, settled snow, and lightning. New `WeatherOptions`,
  `setWeather()`, `getWeather()`, `RenderStats.weather`, and the
  `"weatherChanged"` event.
- **Cloud types**: `CloudsOptions.stratiform`, `towering`,
  `baseDarkness`, `raggedBase`, and `rainShafts`. Each weather state has
  its own clouds: stratus when overcast, dark ragged nimbostratus with rain
  shafts in rain, and cumulonimbus towers with anvils, rain shafts, and
  lightning that lights the clouds from inside in storms.
- **High cirrus**: `CloudsOptions.cirrus` and `cirrusHeightMetres`.
- **Shadows** from terrain (a baked horizon map), trees (a sun shadow
  map), and clouds, each configurable through `ShadowOptions` and
  `setShadows()`.
- **Replacement hooks** for your own assets: `setTreeModel()`,
  `resetTreeModel()`, `setTreeInstances()`, `replaceTexture()`,
  `resetTextures()`, `FloraOptions.speciesRules`, and the `imageToRgba()`
  helper.
- `TreePlacement.ground`: hand-placed trees can stand on the drawn
  terrain, sinking their roots on slopes, as procedural trees do. The
  demo's grove button has a "Ground hand-placed trees" checkbox.
- **Surface options**: `SurfaceOptions` and `setSurface()` for flat-colour
  mode, detail normals, texture scale, and per-material tints.
- `CloudsOptions.resolutionScale`: clouds render at a reduced resolution,
  half by default.
- Mist wind drift and sun scattering.
- New guides: `docs/weather.md`, `docs/shadows.md`, `docs/hooks.md`,
  `docs/biomes.md`, and `docs/frameworks.md` (React, Vue, and Svelte
  components, moved out of the README).
- A rewritten README with a screenshot, a quick start, and clear paths for
  people using the package and people changing it; a documentation index
  (`docs/README.md`); and `CONTRIBUTING.md` with a fresh-setup guide.
- **A frame profiler.** `RenderStats.gpuPassTimesMs` reports GPU time per
  pass (terrain, trees, grass, clouds, sky and fog, water, shadows, tree
  culling, upscale and lens), and `gpuFrameTimeMs` their sum, from
  timestamp queries read back without stalling. The demo lists them in its stats panel.
- **Render, detail, and cloud distances.**
  `RenderQualityOptions.renderDistanceMetres` (terrain, trees, and water
  beyond it are not shaded, hidden by distance fog over
  `renderFadeMetres`),
  `detailDistanceMetres` (distant terrain takes one texture sample per
  material instead of up to eight), and `cloudDistanceMetres` (how far
  clouds are marched, thinning out over `cloudFadeMetres`). `preset` now
  fills in whichever are unset, and the demo has a control for each.
- **A frame-rate cap.** `RenderQualityOptions.maxFrameRate` (default 60,
  `0` for uncapped) renders evenly spaced frames, so a 60 cap on a 120 or
  144 Hz display gives a steady 60 rather than a rate that swings with the
  scene.
- **Dynamic resolution.** `renderScale` renders the scene below the canvas
  resolution, and `dynamicResolution` (on by default, down to
  `minRenderScale`, default 0.5) lowers it when frames arrive late and
  raises it when there is time to spare, to hold the frame rate. A final
  pass upscales with contrast-adaptive sharpening.
  `RenderStats.renderScale` reports the scale in use; the demo has
  controls and shows it.
- `CloudsOptions.temporal`: reuse distant clouds between frames. A
  quarter-size pass marches one sky pixel of every 2 x 2 block, a different
  one each frame, and the rest are reprojected from the previous frame,
  cutting the cost of sky clouds by about three quarters. Off by default;
  a checkbox in the demo.
- `WeatherOptions.lensDrops`: raindrops that land on the lens, refract the
  scene, and run down the screen. Off by default.
- **A project site on GitHub Pages.** A front page with screenshots,
  features, a quick start and the live star count; the three.js and
  Babylon.js guides, rendered from `docs/` at build time; an API
  reference; and the demo, which moves to `demo/` on the site.
  `npm run build:site` builds it into `_site/` and fails on any broken
  link or anchor.
- A Babylon.js guide (`docs/babylonjs.md`) and example
  (`examples/babylonjs/`, `npm run dev:babylonjs`): Babylon.js meshes drawn
  over a VistaWASM world with one shared camera, a right-handed scene, a
  depth-only terrain occluder in its own rendering group, and a heightmap
  mesh built from the raw heights.
- A three.js guide (`docs/threejs.md`) and example (`examples/threejs/`,
  `npm run dev:threejs`): three.js objects drawn over a VistaWASM world
  with a shared camera, a matching sun, and hills that hide them; and
  VistaWASM terrain drawn by three.js, including in browsers without
  WebGPU.
- Benchmarks (`bench/`): production download size and terrain generation
  speed compared with THREE.Terrain and three-terrain, with the method,
  results, and a feature comparison. Run them with `npm run size` and
  `npm run speed` inside `bench/`.
- **Demo.** A link to the GitHub repository; collapsible sections with
  remembered state; value readouts on every slider; one-click weather
  presets; and controls for every option, including weather effects,
  shadows, surface, cloud types, and cloud quality. A "Custom assets"
  panel demonstrates each hook: a custom tree model built in JavaScript, a
  species rule, a hand-placed grove, and texture replacement from an
  image file.
- **Snow biomes.** High ground now rises through `alpineTransition`
  (scree, patchy snow and dwarf shrubs below the snow line) into
  `lowerSnowyPeaks` (snowfields broken by rock) and `upperSnowyPeaks`
  (permanent snow and ice). `biomeAt()` reports them, and the `"biomes"`
  debug view colours them.
- **Landforms.** `FractalTerrainOptions.landform` picks the character of a
  generated map: `"continental"` (the default), `"alpine"`,
  `"rollingHills"`, `"archipelago"`, `"mesaDesert"`, `"fjords"` or
  `"volcanicIsland"`. See `docs/terrain-data.md`.
- Terrain generation progress: `generateFractal()` now emits `"progress"`
  events for its `"tectonics"`, `"drainage"`, `"detail"`, `"erosion"` and
  `"finishing"` phases, with erosion reported at least every 10 %.
- **Demo.** A Landform select, an Advanced terrain sub-section for the
  detail noise, an erosion quality select (default `"high"`), and the time
  of each generation phase in the status line.
- **Ice and arctic biome.** `iceArctic`: glaciers and ice sheets with
  crevasses and blue ice, a tundra fringe of moss, lichen, dwarf shrubs,
  and stones, snow that never melts, and sea ice with drifting floes and
  fast ice on cold coasts. Glaciers fill valleys with smooth ice and are
  removed exactly when the climate warms.
- **Climate temperature.** `BiomeOptions.meanTemperatureCelsius` (-30 to
  35 °C at sea level) drives every biome, cooling 6.5 °C per 1000 m. New
  `temperatureAt(x, z)` returns the mean temperature in °C at a world
  position, or `null` off the terrain.
- **Cold weather.** Rain falls as snow below 0.5 °C at the camera and as
  sleet up to 2.5 °C; cold climates cycle towards snow and clear spells
  and always allow snow; gales lift blowing snow off snowy ground; and
  cold air is crisp and clear.
- Glacier ice and tundra terrain textures, generated at start-up.
  `replaceTexture` accepts terrain layers 0 to 9, and `materialTints`
  accepts 8 or 10 colours.
- **Demo.** A climate temperature slider with an automatic setting, the
  temperature under the camera in the biome readout, and a biome colour
  legend for the biomes debug view.
- **Lens drop controls.** `WeatherOptions.lensDropCount`,
  `lensDropMinSize` and `lensDropMaxSize` set how many drops are on the
  lens in full rain and how large they are, as fractions of the canvas
  height. Small drops bead and evaporate; large ones run down the screen,
  swallow the beads they touch and leave trails. The demo's weather
  section has matching sliders.
- **Map edges.** `FractalTerrainOptions.edges`: `"coast"` (the default)
  rings the land with sea along a natural, wandering coastline, and
  `"open"` lets the land run to the map edge for tiling several maps. The
  demo's terrain section has an Edges select.
- **Rivers, lakes and waterfalls.** Water is routed over the terrain
  from rain, snowmelt and springs: glacier snouts and snow fields feed
  streams, closed basins fill to their spill height as lakes that
  overflow or, in dry climates, stay endorheic and salty. Channels take
  their width, depth and speed from their discharge, meander on gentle
  ground, leave oxbow lakes, fan into deltas at the sea, and drop over
  steps as waterfalls into plunge pools, with a sheet, mist and churn.
  Rivers show rapids, bends and sediment; lakes, rivers and waterfalls
  freeze in cold climates. Banks darken with wet ground and mud, reeds
  grow beside still water, and grass is greener near water. New
  `RiverOptions.snowmelt`, `springs`, `meanders` and `waterfalls`.
- **Painted water.** `setWaterMask(width, height, data)` carves the
  rivers and lakes of a host's mask and draws them like the terrain's
  own; clearing it restores the terrain exactly.
- **Water sound hooks.** `getWaterSounds(x, y, z)` returns the nearest
  river, waterfall, lake shore and surf with their loudness, and
  `getWaterfalls()` lists every waterfall, for hosts that play their own
  audio.
- **Big rivers from beyond the map.** `RiverOptions.inflow` brings water
  in across an open edge: `"auto"` (the default) places one inflow at the
  lowest valley mouth an eighth of the map from the sea, sized from a
  basin ten times the map's land area; `"none"` adds nothing; or list up
  to 8 inflows with a position and a discharge. `getInflows()` returns the
  inflows in use. Big rivers on gentle ground get a flat valley floor
  beside their banks instead of a canal.
- **Small streams at their true size.** Streams narrower than a heightmap
  sample meander at their own wavelength inside their carved trench, get
  crisp muddy, sandy or gravelly banks from a new bank-strip pass, and
  stay continuous faint lines in the distance instead of dashes. Reeds
  line the true banks of slow brooks on coarse maps.
- **River beds.** A gravel terrain material in slot 10: rounded cobbles
  with dark crevices. River banks sort by the flow into gravel, sand and
  mud, with point bars on the inner side of bends and sand at mouths;
  wet stones by the water are dark and glossy. Shallow, fast water shows
  stones on its bed, some breaking the surface with foam rings, and
  powerful, steep reaches (stream power over 300 W/m²) get bigger stones,
  wilder rapids and rock walls. `materialTints` accepts 11 colours and
  `replaceTexture()` accepts terrain layer 10.
- **Green banks.** `RiverOptions.riparian` (0 to 2, default 1) greens the
  ground within 25 to 400 m of rivers, by discharge, and 40 m of lakes:
  moister soil, meadow and thicket in dry country, and more trees.
- **Reflections.** `WaterOptions.reflections`: `"screen"` (the default)
  reflects the terrain, trees and banks on screen in rivers, lakes and the
  sea, falling back to the sky; `"sky"` reflects the sky and clouds only.
- **Dense forests and full grass.** `FloraOptions.density` and
  `GrassOptions.density` now run from 0 to 4; 1 is the old maximum. At 4
  forests and jungles close their canopy and grass covers the ground.
  Every tree and tuft is a point on one jittered world lattice, decided
  identically on the CPU and the GPU. Up to 50,000 trees are placed on
  the CPU and drawn as before; denser forests keep a static far set and
  stream the rest into 64 m tiles round the camera on the GPU, thinned
  with distance and grown so the canopy stays closed. Grass always
  streams, in 16 m tiles; reeds stay on the CPU.
- **Canopy layer.** Beyond `RenderQualityOptions.canopyDistanceMetres`,
  streamed forests give way to a canopy shell over the terrain, with a
  crown-by-crown crossfade, crown cover that closes up at a slant as real
  forest does, and species colours. It appears in screen reflections.
- **Forest floor.** Leaf litter, moss and dappled light now follow the
  canopy above them rather than the biome, so bare slopes in a forest
  stay grassy. `GrassOptions.forestFloor` (default `true`) grows ferns
  and undergrowth instead of grass under dense canopy. Mist gathers a
  little under closed canopy.
- **Vegetation budgets.** `RenderQualityOptions.vegetationDetailMetres`,
  `canopyDistanceMetres`, `maxTreeInstances` and `maxGrassInstances`,
  filled in by the preset. The engine shrinks the detail radius when the
  drawn estimate is over budget, and further while frames are late, before
  dynamic resolution may drop below 0.85. In dense forests the
  mesh distance comes in so at most a twentieth of `maxTreeInstances` are
  full meshes.
  `RenderStats.floraInstances` and `grassInstances` report the estimated
  drawn counts, and `gpuPassTimesMs.generation` times tile generation.
  The demo has density sliders to 4, a forest floor checkbox, and
  vegetation detail and canopy distance selects.
- **Tree triangle budget.** `RenderQualityOptions.maxTreeTriangles`
  (1, 2.5 or 5 million by preset, no limit at `"offline"`). The cull
  pass's draw counts are read back a frame or two late, never stalling;
  over budget the mesh distance comes in, so the furthest meshes become
  impostors first and no tree is dropped. Tree shadow casters keep to a
  quarter of the budget. `RenderStats.treeTriangles` reports the total.
- **Full-cover grass radius.** `RenderQualityOptions.grassDetailMetres`
  (25, 45, 70 or 120 m by preset) sets how far grass is at full cover.
- **Split tree timing.** `RenderQualityOptions.splitTreeTiming` times the
  trees pass as canopy meshes, understorey meshes and impostors, in
  `gpuPassTimesMs.treeMeshes`, `understorey` and `treeImpostors`.
  `scripts/visual-check/fixed-scene.mjs --jungle` measures a dense jungle
  from three fixed cameras, and `--split` breaks the trees pass down.
- The demo has tree triangle budget and full grass radius selects, a
  split tree timing checkbox, and the tree breakdown and triangle count
  in its GPU profile.
- `FlyCameraControls.lookAt([x, y, z])` turns the camera towards a point.
  The demo has snowmelt and meander sliders, springs and waterfalls
  checkboxes, a button that jumps to a waterfall, and a water sounds
  readout; a River valley preset, a button that jumps to the main river,
  an inflow select with a discharge slider, a green banks slider and a
  reflections select.
- **Rock outcrops and scree.** Bare rock shows where the soil is thin:
  on convex shoulders and ridge crests, on the steepest ground, in the
  frost-shattered ground above the trees, and where harder beds of the
  bedrock cross a slope, so outcrops run as broken crags and ledges along
  the contours (tight, level bands on mesa desert). Scree, a new twelfth
  terrain material with its own texture layer (`terrainAlbedo` and
  `terrainNormal` layer 11), lies below outcrops down to where the slope
  eases, and covers the alpine transition between the snow patches. New
  `SurfaceOptions.rockiness`, 0 (deep soil everywhere) to 2 (rocky).
- **Rock that reads as stone.** Rock faces show jointed blocks and
  bedding ledges from 50 m to 2 km, deep cracks shaded from the sky,
  sparse lichen on moist, sunward, gently inclined rock, and dark water
  streaks down faces steeper than 60 degrees. Within 120 m, rock and
  scree take parallax, and close up a finer scale keeps them crisp.
  Outcrop edges are lobed and frayed, and turf thickens at the rock's
  lip.
- **Boulders and talus.** Six procedural fractured blocks, 0.3 to 3 m
  across (many small, few large, the largest furthest down the cone),
  streamed on the GPU in 32 m tiles below outcrops and on scree, sunk
  into the ground and leaning with the slope, with levels of detail and
  shadows within 150 m. Never in water, in any drawn channel, on sand,
  snow or glacier, or on the crag itself; trees and grass leave room for
  them. New `SurfaceOptions.boulders` and `boulderDistanceMetres`
  (default 300 m), and `gpuPassTimesMs.boulders`.
- The demo's surface section has a rockiness slider, a boulders checkbox,
  a boulder distance select and scree in its texture list.
- **Grown trees.** Every species now grows by space colonisation inside
  its own crown envelope (lobed, layered or conical), with branch radii
  from the pipe model, so crowns show real forks, open sky between limbs
  and the thin twigs at their tips. Trunks flare into roots that reach
  below the ground; rainforest emergents stand on buttresses and swamp
  cypresses raise knees through the water. Growth takes about 110 ms in
  WASM before the first frame.
- **Leaf clumps.** Oak, jungle and cypress-scale leaves, pine and spruce
  needle bundles, palm fronds and fine leaves are generated as clumps
  of individual leaves with their own normal maps, so crowns show leaf
  silhouettes near the camera and shade like foliage, not flat cards.
  Bark has species colours, furrows and moss on the shaded side.
- **Tree variants, ages and lean.** `FloraOptions.variantsPerSpecies`
  (1 to 4, default 4) grows distinct shapes per species. Each tree also
  takes an age class from its ground (young, mature, old, or wind-cut
  krummholz near the tree line) and leans away from open water and down
  steep slopes, so neighbours differ. The first variant grows before the
  first frame and the rest over the frames after it. New
  `RenderStats.treeGrowthMs` and `treeBakeMs`.
- **Tree levels of detail.** Full meshes within 50 m, lighter meshes
  (under 45 % of the triangles) to 150 m, and impostors beyond, each
  hand-over a dithered cross-fade. Impostors have nine views (eight
  around and one from above) with normal maps, and keep their foliage
  density at every mip level instead of thinning into skeletons.
- **Layered wind.** Trunks sway about their base, limbs bob about their
  own pivots and leaves flutter, bounded to 8 % of the tree's height and
  still at the ground. Meshes, impostors and shadows move together.
  Models from `setTreeModel()` get wind data from their shape.
- The demo's vegetation section has a "Tree variants" slider and a
  "Tree showcase" button that plants one tree of each species in front
  of the camera and frames them.
- **Weather presets.** Weather is one table of presets that every system
  reads as one blend: sky and haze, clouds, mist, rain and snow, wind on
  trees, grass and the sea, sea state, wet and snowy ground, and light.
  Thirteen built-in presets (`clear`, `fewClouds`, `partlyCloudy`,
  `brokenClouds`, `overcast`, `mist`, `fog`, `lightRain`, `rain`,
  `heavyRain`, `storm`, `snow`, `blizzard`); the seven older states keep
  their values. `WeatherOptions.presets` adds presets or changes fields
  of built-in ones, with `extends`, successor weights, durations and a
  climate; every field is range-checked and unknown fields are rejected.
  `WeatherKind` accepts custom names. New `getWeatherPresets()`.
- **Regional weather.** A drifting 128 x 128 weather map, 64 km across by
  default (`regionSizeKm`), varies coverage, precipitation, storm cells
  and humidity across the land, so a storm can pass in the distance while
  the camera is in sun. Rain falls only under thick cloud, and storm cells
  grow the towering clouds. `regional: false` gives the same weather
  everywhere. New `weatherAt(x, z)` for gameplay and audio.
- **Wet ground that dries.** Wetness, puddles and snow depth follow the
  rain and snow where they fell and dry with sun, wind and warmth, slower
  under trees and in patches. Puddles form in flat hollows, mirror the
  sky with raindrop rings, and go last. Wet bark darkens. New
  `advanceWeather(seconds)` runs the weather and the ground forward at
  once.
- **Humidity and turbidity haze.** Presets' `humidity` and `turbidity`
  whiten and thicken the haze, from deep, crisp alpine blue to hazy
  tropical air. `AtmosphereOptions` stay multipliers on top.
- **Light under cloud.** The clouds between the camera and the sun dim
  the direct light; overcast softens shadows, widens terrain penumbrae,
  flattens and dims the sky light, and fades sun glitter.
- **One wind.** A gust front sweeps across the land, shared by trees,
  grass, rain, cloud drift and the water. The sea state follows the wind:
  wave height from wind speed and fetch, whitecaps from 4 m/s, blown
  spray from 15 m/s, and waves that turn to the wind.
- **Time of day.** `setTimeOfDay()` moves the sun by the NOAA solar
  position equations for a latitude and day of the year, and
  `getTimeOfDay()` reports it with sunrise and sunset. With auto-cycle,
  the weather tends to clear before sunset, so storms end in wet, golden
  evenings.
- The demo's weather section lists every preset, edits the selected one
  ("Edit preset", with "Reset preset"), switches regional weather, skips
  ahead 10 minutes, and runs the time of day; the stats panel shows the
  weather at the camera.
- `GpuPassTimes.surfaceWeather` times the wet ground's compute pass.
- **Uneven, lumpy cloud bases.** `CloudsOptions.baseVariation` (default
  0.07) sets each low cloud's base at its own level, tens of metres from
  its neighbours, with rims that curl up gently; `baseLumpiness` (default
  0.6) erodes the undersides into soft, cotton-wool lumps while thin
  fringes keep their wisps. Bases stay mostly flat and never spike;
  flat sheets keep one base.
- **Mid-level clouds.** `CloudsOptions.altocumulus` (rippled rows of
  cloudlets, a "mackerel sky") and `altostratus` (a grey veil that turns
  the sun into a watery disc with a soft corona), at `altoHeightMetres`
  (default 4200 m) and drifting at `altoSpeed`, along the cloud wind
  veered 20 degrees. The layer is one cheap 2D layer in the cloud pass,
  and shades the cumulus below and the ground. Both default to 0.
- The six new cloud fields are preset fields too (`baseVariation`,
  `baseLumpiness`, `altocumulus`, `altostratus`, `altoHeightMetres`,
  `altoSpeed`), range-checked like the rest. The regional weather's
  coverage raises or lowers the mid-level layer by up to 30 %.
- The demo's clouds section has sliders for the bases and the mid-level
  layer, and "Mackerel sky" and "Veiled sun" buttons.
- **Map export.** `engine.exportMap(kind, options?)` reads back every map
  the renderer builds: height, biome, water, water depth, flow,
  discharge, the twelve material weights, slope, normals, occlusion,
  temperature, moisture, and tree and grass density. Each comes with an
  encoding (units, scale, range, and a legend naming every biome, water
  kind and material). Maps are at the terrain's own size, or resampled
  in Rust up to 2048 x 2048, the largest terrain: bicubic, bilinear for normals and
  materials, and nearest for biomes and water.
- **Tree export.** `engine.exportTrees(options?)` lists every tree on
  the map or in a region, exactly: the whole lattice at the current
  density, including the trees streamed near the camera, or the
  hand-placed trees, each standing on the ground by its roots, with its
  species, variant, size, rotation and colour.
- **Map files and bundles.** `encodePng()` writes 8 and 16-bit grey,
  RGB, RGBA and palette PNGs directly; `encodeRaw()` writes Float32 or
  scaled Uint16; `treesToCsv()` and `treesToJson()` write trees; and
  `exportBundle()` and `downloadBundle()` write every map, the trees
  and the options into one zip with a versioned `manifest.json`. No new
  dependencies: the PNG and zip writers are small and built in.
- `engine.getOptionsSnapshot()` returns every option in effect, and the
  fractal options of a generated terrain, to rebuild the same scene.
- The demo has an Export section: a map, size and format, and buttons
  to export a map, the trees as CSV, or a bundle, with the file sizes and
  time taken.
- **Heightmap images.** `engine.loadHeightmapImage(source, options)`
  loads a PNG at 8 or 16 bits a channel, or any image the browser
  decodes, at 8 bits with a warning. A 16-bit PNG from `encodePng()`
  brings back its height range. `decodePng()` decodes every PNG colour
  type and bit depth, all five filters and Adam7 interlacing, checking
  every CRC.
- **Painted biome maps.** `engine.setBiomeMap(map)` paints biomes onto
  the terrain. Painted biomes are absolute, with irregular borders
  warped by up to `borderSamples` samples and ground textures blended
  across them. Painted ice gets glaciers and permanent snow, and painted
  volcanic ground its heat. Ocean painted above sea level is classified
  as usual, with a warning. `biomeMapFromImage()` reads exported palette
  PNGs by index and other images by the nearest legend colour in
  CIELAB, or your own legend.
- **Water masks from images.** `waterMaskFromImage()` reads the exported
  water map's lake and river colours, with brightness setting a river's
  strength, or grey values.
- **Vegetation density masks.** `engine.setVegetationMasks({ trees,
  grass })` scales where trees and grass grow: 0 none, 128 unchanged,
  255 twice as dense, never denser than density 4. The grass mask scales
  the tufts and the ground's distant grass sheen alike.
  `densityMaskFromImage()` reads one from an image.
- **Bundle import.** `loadBundle(engine, source)` loads a bundle and
  recreates the scene exactly, from the heights before any carving, the
  options and the painted maps. `loadTerrainFromImages()` loads a
  heightmap image with its painted maps in one call. The readers are
  built for untrusted files: sizes, inflated data, zip entries and the
  manifest are bounded, and every malformed file throws `INVALID_DEM`
  with a clear message. `engine.getPaintedMaps()` returns the painted
  maps in effect, and `exportMap("sourceHeight")` the heights before
  any carving.
- `RawHeightmapOptions.landform` sets a loaded heightmap's landform,
  which shapes its bedrock.
- The demo has an Import section: file pickers for a heightmap image
  (with its scale and height range, filled in from a 16-bit PNG), a
  biome map, a water mask, tree and grass masks, and a bundle, with
  warnings in the status line.
- **Demo overlays.** A View section shows and hides the stats and FPS
  panel, the minimap, the controls hint, the status line and the whole
  side panel, with the keys `1`, `2`, `3`, `4` and `0`, which do nothing
  while typing. An error brings a hidden status line back. A Show
  controls button brings a hidden panel back. The choices are
  remembered, and the panel starts hidden on phones. See
  [`docs/demo.md`](docs/demo.md).
- **Demo loading panel.** While the engine starts, terrain generates or
  a map imports, a panel over the view shows a progress bar through the
  generation phases, with what each is doing. Errors appear in it, with
  a Close button. See [`docs/demo.md`](docs/demo.md#loading).
- **Demo Paint tab.** Explore and Paint tabs. Paint is a 2D editor for
  256, 512 or 1024 square paintings, started blank, from the current map
  or from imported images or a bundle, with raise, lower, smooth,
  flatten, noise and erode brushes; a biome brush with a palette; lake,
  river and erase water brushes; and tree and grass density brushes,
  each with size, strength and falloff. It has tiled undo and redo, zoom
  and pan with the wheel, keys or two fingers, and pen pressure. Render
  in 3D loads it into the engine and frames it, or it renders after each
  stroke. Paintings save and open as bundles, and each layer exports as
  a PNG that imports back unchanged.
- `scripts/visual-check/demo-capture.mjs` drives the demo in headless
  Chromium: it checks the overlays, paints, renders, undoes, saves and
  reopens a painting, paints with touch, and times strokes on a slowed
  CPU. `scripts/visual-check/offscreen-canvas.js` holds the WebGPU set-up
  it shares with the visual-check page.
- Vitest runs the demo's tests in `demo/tests`: brushes, history, the
  paint document and the shortcuts.
- The error code `WASM_LOAD_FAILED`: the WASM module or its JavaScript
    glue could not be fetched, compiled or started. The message names the
    module URL, and the browser's own error is kept in `details`.
- The limits the JavaScript boundary enforces are exported:
    `MAX_OPTIONS_DEPTH` (8), `MAX_WEATHER_PRESETS` (64),
    `MAX_PRESET_NAME_LENGTH` (64), `MAX_LEGEND_COLOURS` (256) and
    `MAX_IMAGE_SIDE` (8192).
- `SECURITY.md`: supported versions and how to report a vulnerability
    privately through GitHub's private vulnerability reporting.
- `docs/security.md`: the threat model, what the library validates, the
    limits it enforces, the Content Security Policy a host page needs
    (`script-src 'self' 'wasm-unsafe-eval'`), why no cross-origin isolation
    headers are needed, and the advisory status of the dependencies.
- Validation and deterministic fuzz tests for every entry point that
    takes input from JavaScript.
- **GPU profile page** (`bench/gpu/`, `npm run gpu` in `bench/`). It
  renders every budgeted scene (the fixed scene clear, in rain and over
  ice, with and without pinned grass; the jungle close-up, clearing and
  hillside; the meadow at grass densities 0.5 and 4 from 4 and 25 m; and
  the five sky views) at 1920 x 1080, render scale 1 and dynamic
  resolution off. It records the median and 90th percentile of every
  pass and of the whole frame over 120 frames, shows them in a table,
  and offers them as JSON naming the browser, adapter, canvas size and
  library version. Without timestamp queries it says so and times whole
  frames. `scripts/visual-check/profile-check.mjs` runs it headless.
- **`docs/performance.md`:** how to run the profile page on your own
  devices, every budget beside its scene, how the estimates so far were
  made, and the results.
- **One shared scene module,** `scripts/visual-check/scenes.mjs`, used by
  both the profile page and `fixed-scene.mjs`, which gains `--sky`
  (the sky views over the sea at (0, 40, -4000), printed with
  hyphenated names such as `rain-deck`) and `--meadow=<density>` (the
  meadow from 2 to 60 m up).
- **Natural rivers.** `RiverOptions.meanderMaturity` (0 to 1, default
  0.5): how long meanders have been developing, from young, gentle bends
  to mature loops with neck cut-offs and oxbow lakes. Wide lowland rivers
  now migrate by bank erosion, leaving levelled floodplains, scroll-bar
  swales and oxbows that are shorter and shallower with age.
- **Braided rivers.** `RiverOptions.braiding` (0 to 1, default 1): steep,
  wide rivers on open valley floors split into two to four threads that
  wander, merge and split across a gravel belt.
- **Eddies.** `WaterOptions.eddies` (0 to 1, default 1): slowing and
  reversed water, eddy lines and turning vortices on the inner bank below
  bends, beside joins, below falls and behind stones, and small boils in
  fast water. Changing it does not rebuild rivers.
- **Deltas** now branch: distributaries split at mouth bars into up to
  eight unequal arms over the fan.
- The demo has "Meander strength", "Meander maturity", "Braiding" and
  "Eddies" sliders, and a "Meandering lowland" preset.
- **Riparian plants.** A band of plants lines every drawn channel: tufts
  leaning over the water, tall herbs and ferns behind them, and scrub 2
  to 6 m tall in clumps with gaps where the water shows, chosen by biome
  and climate, thinned on scoured and steep banks and thickened by slow
  water. Far off, a deeper green lines the water where the scrub thins
  out.
- **Shaped banks.** Banks are meshes with a wet margin, a face and a turf
  lip: cut banks on the outside of bends stand steep with an overhanging
  lip, inner banks shelve gently.
- **Real stones.** Fast streams carry boulder-mesh stones sized by the
  force of the flow, with cobbles along their edges; the water breaks on
  the same stones.
- **Stepped water.** Streams on slopes of 6 to 30 % drop from level pool
  to pool behind their largest stones, with white water below each lip.
- **Seeing into the water.** `WaterOptions.refraction` (0 to 1, default
  1): the bed under shallow water bends with the ripples, and caustics
  play over it in sunlight. 0 turns both off. Changing it does not
  rebuild rivers.
- **Catchment colour.** Rivers draining bog run tea-brown; rivers below a
  glacier run milky turquoise.
- **River mouths.** Estuaries flare smoothly, sand bars flank wide
  mouths, rivers fade into the sea, and each big river's colour spreads
  out to sea in a plume.
- The demo has a "Refraction and caustics" slider, and "Braided valley"
  and "Delta coast" presets.
- **GPU errors reach JavaScript.** A `"gpuError"` event carries each GPU
  validation or out-of-memory error the engine's own error scopes did
  not catch or that a terrain's upload raised, and the new `GPU_ERROR`
  code rejects an engine creation or a GPU erosion that failed. `"deviceLost"` now fires once, with the
  browser's reason, from whichever call first finds the device lost,
  not only from the `start()` loop.
- `MAX_RAW_HEIGHTMAP_BYTES`, `MAX_DEM_BYTES`, `MAX_TERRAIN_SIDE`,
  `imageSize()` and `BUNDLE_LIMITS.decodedBytes` are exported, and
  `fetchDemBytes()` and `loadDemFromUrl()` take `maxBytes`.

### Changed

- The demo reuses distant clouds between frames by default (**Reuse
    distant clouds between frames** starts ticked), which cuts the cost
    of sky clouds by about three quarters. The library default of
    `CloudsOptions.temporal` is still `false`.
- Bundles are version 2: they add `source-height.f32`, the painted maps
  and the terrain's landform, so they load back exactly. Version 1
  bundles still load, from their final heights, with a warning.

- **Grass is on by default,** at density 0.5. The default scene's frame
  costs the same with it (about 3.3 ms at 1080p on a mid-range GPU), and
  a meadow near the camera about 0.7 ms. Pass `enabled: false` to turn
  it off.
- **Grass density means cover, not counts.** `GrassOptions.density` 0.5
  (the default) is now a natural, dense meadow whose tufts cover at
  least 70 % of the ground near the camera; 1 a lush, taller meadow; and
  4 long, dense grass on every point. The old calibration, where 0.5 and
  1 gave a few hundred scattered tufts on a meadow, is dropped on
  purpose: set a lower density for sparser grass.
- **Grass near-dense, far-cheap.** Full cover reaches
  `grassDetailMetres`; beyond it tufts thin and widen as before, then hand
  over to the ground by `viewDistanceMetres`: grass-covered terrain takes
  on their colour, a view-dependent sheen and a fine fuzz, so a meadow
  still reads as grass at 200 m without tufts. Within 15 m a tuft is two
  crossed quads of seven blades; beyond, one card turned to the camera.
  Grass is lit per vertex. On steep ground it thins smoothly from 30 to
  50 degrees, tuft by tuft, and how readily it grows is read between
  texels, so steep meadows show no bands along the contours. A meadow at
  density 0.5 costs about 0.7 ms at 1080p on a mid-range GPU, and at 4
  about 1.9 ms.
- **Detail gives way before resolution.** With streamed vegetation,
  dynamic resolution lowers the render scale only to 0.85, then raises
  detail pressure (smaller near radii, more impostors) to its most, and
  only then lowers the scale further. Without vegetation to shed, the
  scale falls as before.
- Each species draws at most 24,576 trees as full meshes, host-placed
  trees included; the rest are impostors.
- **Trees grow where their species would.** Each species has a niche
  (temperature, moisture, steepest slope, water affinity, shade and
  exposure tolerance). Drainage, shaded slopes (facing away from the
  sun), riverbanks, exposed ridges and the tree line shape where it
  grows. Trees gather in groves with young trees around their parents,
  and are stunted near the tree line and on exposed ridges. Nothing grows
  in any drawn channel, on gravel bars, sand (except palms), rock, lasting
  snow, glacier, volcanic heat or the skirt. Candidates are jittered, so
  trees no longer stand on the sample grid, and the instance cap thins
  the whole forest evenly instead of in stripes. Steep hillsides beyond
  each species' slope limit (32 to 45 degrees) now carry fewer trees.
- River width now follows discharge instead of a fixed rule, rivers
  depend on the climate's rain and snow, and river-bed samples are no
  longer classified as sand or mud; wet banks show mud instead.
- Terrain generation reports a `"rivers"` progress phase.
- **Faster first frame.** Pipelines and procedural textures are created
  when the scene needs them: only the terrain layers, tree species and
  cloud noise in use are baked, and pipelines the first frame does not
  draw are warmed up one per frame afterwards. The default scene's first
  frame arrives in about half the time under software WebGPU.
- **Cheaper rivers.** The channel carve visits each sample once, and
  straight, uniform runs of river ribbon are merged: the carve runs about
  2.6 times faster and ribbons use about half as many vertices.
- **Falls at their size.** Plunge pools scale with discharge, trickles
  under 0.05 m³/s fall as whitewater with no sheet, mist or pool, and
  falls close together form one cascade. `getWaterfalls()` lists a
  cascade as one waterfall, from its first lip to its last foot.
- Open-edge maps now have a big river by default (`inflow: "auto"`), the
  ground by rivers is greener (`riparian: 1`), and water reflects the
  scene (`reflections: "screen"`). Set `inflow: "none"`, `riparian: 0` or
  `reflections: "sky"` for the previous look.

- **Fractal terrain is geology-led.** Continents with an exact land
  fraction and uplifted ranges are carved by a stream-power model into
  dendritic valleys and ridge spurs, with flat valley floors and glacial
  troughs, then detailed and eroded. Noise is seeded gradient noise instead
  of value noise, and features have real sizes in metres. Maps have
  plains, coastlines and ranges instead of a field of spikes. The same seed
  gives a different map from earlier builds; `generatorVersion` is now
  `vistawasm-fractal-0.2.0`.
- `NoiseOptions` now controls the detail layer on top of the landform, and
  `TerrainShapeOptions` applies in units of the landform's relief.
- The coast is generated at `seaLevelMetres`, and `verticalScale`
  stretches heights about sea level.
- **Mountains stand as high as real ones on real map sizes.** Relief was
  capped at a quarter of a range's width, so on a 6 km map alpine peaks
  reached 700 to 1,050 m and read as green domes. Each landform now has
  its own steepness allowance (alpine and fjords 0.55, archipelago and
  volcanic islands 0.4, continental 0.35, mesa desert 0.3), range belts
  stand on a massif that stream power carves, lie within the land rather
  than along its shores on coast-ringed maps, and keep steep river
  profiles. Glacial troughs are over-deepened in steps, with shallow
  basins behind them, without lowering the ridges beside them. At 512 x 512
  and 12 m, alpine maps now reach p99 heights of 1,500 m and more with
  snowy peaks on every seed, fjords 1,100 m and more, and continental
  maps peak between 900 and 1,400 m.
- **Sea ice looks like pack ice, not floor tiles.** The single mosaic of
  equal plates is replaced by floes at three scales (600 m, 120 m and
  25 m), so their sizes span orders of magnitude, with rounded, irregular
  edges; long leads of open water that meander across the sea and close
  up as the pack tightens; pressure ridges along some floe boundaries;
  grey slush between floes in a close pack; and white, blue-grey and
  thinly snowed floes. Open water under solid ice is no longer shaded.
- Terrain deep under opaque water skips its shading, since the water
  hides it; the terrain pass is about 5 % faster in the fixed test scene.
- Generated maps are ringed by sea by default (`edges: "coast"`), for
  every landform, so land no longer runs into the map edge. Each landform
  keeps its land fraction. `edges: "open"` gives the previous heights bit
  for bit.
- **Erosion** is a virtual-pipe shallow-water model that cuts gullies,
  aggrades valley floors and builds alluvial fans, plus talus-angle
  thermal erosion with soil creep. It runs at half and then full
  resolution, on the GPU in browsers with the CPU as a fallback. Unset
  `ErosionOptions` fields take the landform's defaults, unset iteration
  counts follow `quality`, and the quality caps are now 120, 240, 400 and
  5000 iterations.
- Fractal and erosion options are validated with messages that name the
  field and its valid range.
- The automatic snow line (`BiomeOptions.snowLineMetres` unset) is now at
  least 400 m above sea level, so low hills no longer turn white.
- Terrain vertices are 36 bytes instead of 40: the normal is
  octahedron-encoded and the ten material weights are packed into twelve
  `unorm8` slots, so the streamed terrain mesh uploads faster (see
  `docs/architecture.md`).
- `FrameUniforms` grows to 784 bytes, and render shaders gain the surface
  texture at `@group(1) @binding(12)`.
- Heavy sleet and snow now darken the sky to a full overcast, as heavy
  rain does.

- The default `"balanced"` render preset now draws terrain beyond 2 km with
  one texture sample per material, and marches clouds to 60 km (from 90
  km). In testing this saved about 12 % of GPU time with no visible
  difference; `preset: "offline"` restores the previous behaviour.

- Volumetric clouds are rebuilt: an adaptive march that refines cloud
  edges, multiple-scattering lighting with bright tops and darker bases,
  distance-aware detail, and a stable dither. They no longer look grainy
  or like cotton wool, and they form rounded domes rather than columns.
- Rendering is linear HDR with ACES tone mapping and a single-scattering
  sky model shared by every shader; haze and mist are applied in a
  depth-aware composite pass.
- Water extends to the horizon instead of stopping at the terrain edge.
- Animation uses a real-time clock instead of a frame counter, and wind
  drift is integrated over time, so changing the wind never makes clouds,
  mist, or currents jump. The time step is smoothed, and a pause longer
  than 0.2 s (a background tab) no longer jumps the scene forward.
- Overcast, rain, and storm skies march cloud lighting with 3 samples
  instead of 5, and sky lighting skips a second sky evaluation that did
  not change the result.
- `setWater`, `setFlora`, `setGrass`, `setClouds`, `setMist`, and every
  new setter validate their input at the JavaScript boundary.
- **Forests back on steep slopes.** Each species' steepest slope is
  higher (oak 38°, pine and spruce 45°, shrub 50°) and slope thins trees
  only over the last quarter of the range; a shrub understorey joins
  wooded biomes above 30°. With rivers off, valley floors are still
  wetter than ridges: a D8 drainage area over the heightmap stands in
  for the river network.
- `FloraOptions.maxInstances` and `GrassOptions.maxInstances` accept up
  to 4,000,000.
- Smaller, faster builds: release builds use link-time optimisation, one
  codegen unit, size optimisation, and `panic = "abort"`; shaders are
  minified at build time, `common.wgsl` is embedded once instead of once
  per shader, and each shader module is compiled once.
- The minimum Rust version for building from source is 1.87, which wgpu
  30 requires. The development notes list exact tool versions and a
  fresh-setup sequence.
- **Faster river build, bounded bank strips.** The river build buffers,
  join tests, simplification, drainage order and distance fields do less
  work for the same output: the `"rivers"` phase on a mesa desert with
  open edges and a 123 m³/s inflow fell from about 300 ms to about
  270 ms in the browser. Bank strips beside small streams merge rows
  where a stream runs straight, and a map holds at most 300,000 strip
  vertices: over that, the narrowest, slowest streams furthest from the
  centre are drawn with fewer rows first (a flat coastal plain had
  466,000).
- **Rock follows the soil, not the slope alone.** Rock used to be a pure
  function of slope and relative height, which painted smooth grey
  blotches on mountainsides. The same total of rock now lies in
  structured places, and forests on steep slopes keep their trees.
  `materialTints` takes a twelfth colour, for scree; lists of 8, 10 or 11
  still work.
- The terrain vertex's river flag byte, which no shader read, now holds
  the canopy layer's lift, freeing material slot 11 for scree.
- **`meshDistanceMetres` hands over at 150 m at most.** Trees draw as
  full meshes to 50 m (or a third of `meshDistanceMetres`), lighter
  meshes to 150 m (or `meshDistanceMetres`, if nearer), and impostors
  beyond. The nine-view impostors look like the meshes from there, and
  pay for the richer near trees.
- **Roots from the species table.** How far trees reach to find the
  lowest ground when they are grounded now comes from one table of
  trunk radii, shared with the grown trunks, instead of the old meshes.
- The distant canopy layer takes each species' colour from its baked
  impostors, so it matches the trees it stands in for.
- Tree meshes have a new vertex layout (56 bytes, with a wind pivot).
  `setTreeModel()` still takes the same arrays.
- **Weather reads the camera's place.** `getWeather()`, falling rain and
  snow, lens drops (rain only) and lightning use the weather where the
  camera is, and `RenderStats.weather` and `"weatherChanged"` report the
  dominant preset there. Wetness and settled snow come from the ground
  under the camera instead of one value for the whole world.
  `stateDurationSeconds` is the default duration for presets that set
  none, and `allowSnow` works with the presets' climates.
- The per-terrain surface, banks and cover textures are one three-layer
  texture array, which frees two texture bindings for the weather.
- **Cloud bases.** Volumetric low clouds now have uneven, lumpy bases
  and a softly shaded grey underside by default. Set `baseVariation: 0`
  and `baseLumpiness: 0` for the old flat, wispy bases (the underside
  shading stays). The lowest bases still rest at `heightMetres`.
- **Mid-level cloud in the presets.** Built-in presets now carry
  altocumulus and altostratus: a little altocumulus from `fewClouds` to
  `storm`, and an altostratus veil in `overcast`, mist, fog, rain, snow
  and storms. Override `altocumulus` and `altostratus` with 0 to keep a
  preset's old sky.
- `FrameUniforms` grows to 960 bytes (`clouds5` and `alto` appended).
- The published JavaScript in `dist/` carries no comments; the
    declarations keep every doc comment. The one exception is
    `dist/import-glue.js`, whose comment tells Vite to leave the glue's
    run-time URL alone.
- The TypeScript wrapper now rejects these inputs before they reach the
    engine. Each throws a `TypeError` for the wrong type, or a
    `VistaWasmError` with code `OPTIONS_INVALID` and the valid range:
    - any options argument that is not an object (every setter,
        `createVistaEngine()`, `generateFractal()`, `loadRawHeightmap()`,
        `loadDemFromArrayBuffer()`, `fetchDemBytes()`, `exportMap()`,
        `exportTrees()`, `exportSnapshot()`, `initialiseVistaWasm()`,
        `loadBundle()`, `loadHeightmapImage()`, the image readers,
        `encodePng()`, `encodeRaw()`, `exportBundle()` and
        `attachFlyCameraControls()`);
    - options with a `__proto__`, `constructor` or `prototype` key at any
        depth, or nested more than 8 levels deep (which includes options
        that contain themselves);
    - `WeatherOptions.presets` that is not a plain object, holds more than
        64 presets, or has a name that is not 1 to 64 characters long, and
        a `WeatherOptions.state` that is not a string of 1 to 64
        characters;
    - `loadRawHeightmap()` and `loadDemFromArrayBuffer()` bytes that are
        not an `ArrayBuffer`, and a `fetchDemBytes()` URL that is not a
        string or a `URL`;
    - `resize()` widths and heights that are not numbers from 0 to 8192,
        and a `devicePixelRatio` that is not over 0 and at most 8 (a
        non-finite ratio used to become 1), checked before the canvas
        changes;
    - `setTreeModel()` with more than 65,536 vertices, plain arrays that
        hold anything but numbers, and plain index arrays with values that
        are not whole numbers from 0 to 4,294,967,295;
    - `setTreeInstances()` trees that are not objects, or whose position,
        scale, rotation, tint or dryness is not a number;
    - a `replaceTexture()` layer that is not a whole number from 0 to 255,
        and texels that are not exactly 512 x 512 x 4 bytes;
    - a `setDebugView()` value that is not a `DebugView`, and an `on()`
        event name that is not a `VistaEventName` or a listener that is not
        a function;
    - an `exportSnapshot()` `mimeType` that is not an `image/…` type, and
        a `quality` that is not from 0 to 1;
    - `encodePng()` and `encodeRaw()` maps whose width and height are not
        whole numbers from 1 to 2048, or whose channels are not 1 to 12,
        and legends with more than 256 entries, or an index that is not
        a whole number from 0 to 255, or a colour that is not three finite
        numbers;
    - `treesToCsv()` and `treesToJson()` trees whose fields are not
        numbers, or whose species is not a `TreeSpecies`;
    - a `biomeMapFromImage()` legend of more than 256 colours;
    - `imageToRgba()` sizes that are not whole numbers from 1 to 8192;
    - `computeHeightmapPixels()` and `exportTerrainObj()` metadata whose
        width and height are not whole numbers, a `colourMode` that is not
        `"grayscale"` or `"hypsometric"`, bytes that are not a
        `Uint8Array`, a `metresPerSample` that is not finite, and a
        `maxSamplesPerSide` that is not a whole number up to 2048;
    - `attachFlyCameraControls()` options with a number that is not
        finite, an `initialPosition` that is not three finite numbers,
        fields of view outside 1 to 179 degrees, or a minimum field of view
        above the maximum; and `setPosition()` or `lookAt()` points that
        are not three finite numbers.
Input that was accepted before and is now rejected with an
`OPTIONS_INVALID` error naming the field and its valid range:

- `render` size and `resize()`: width and height above 8192 CSS pixels
    (valid: 1 to 8192). The drawing buffer is clamped to 8192 device pixels
    a side instead of failing.
- `CameraOptions`: `position` and `target` beyond ±10,000,000 m;
    `nearMetres` and `farMetres` above 1e9; a `rollDegrees` that is not
    finite; a negative `minimumHeightAboveTerrainMetres`.
- `setSun()` now validates like `create()`: `elevationDegrees` outside -90
    to 90, `intensity` outside above 0 to 100, a non-finite
    `azimuthDegrees`.
- `setAtmosphere()` and `create()`: `rayleighStrength` and `mieStrength`
    outside 0 to 100, `hazeDistanceMetres` outside above 0 to 1e9,
    `exposure` outside above 0 to 100, `skyTint` components outside 0 to 4
    (none of these were checked by `setAtmosphere()` before).
- `WaterOptions`: `foam`, `waves.steepness` and `waves.directionalSpread`
    outside 0 to 1; `seaLevelMetres` beyond ±100,000.
- `FloraOptions`: `speciesVariation` and `windStrength` outside 0 to 1;
    `meshDistanceMetres` above 5000.
- `GrassOptions.viewDistanceMetres` above 1000.
- `CloudsOptions`: `evolution` and `density` outside 0 to 1.
- `MistOptions.sunScattering` outside 0 to 1.
- `BiomeOptions.volcanism` outside 0 to 1.
- `WeatherOptions`: `stateDurationSeconds` above 86,400,
    `transitionSeconds` outside 0 to 86,400, more than 64 custom `presets`,
    preset names (in `state`, `presets`, `extends` and `next`) longer than
    64 bytes, and objects of values by name with more than 256 entries.
- `RenderQualityOptions`: `maxClipmapLevels` outside 1 to 12,
    `floraDensityScale` outside 0 to 4, `maxFrameRate` outside 0 to 1000,
    `renderDistanceMetres`, `detailDistanceMetres`, `cloudDistanceMetres`,
    `canopyDistanceMetres`, `renderFadeMetres` and `cloudFadeMetres` above
    1e9, `vegetationDetailMetres` above 5000.
- `FractalTerrainOptions`: `horizontalScaleMetres` above 10,000,
    `verticalScale` above 100, `baseHeightMetres` and `seaLevelMetres`
    beyond ±100,000, `noise.lacunarity` above 8, and `shape` fields outside
    0 to 1 (they were clamped before).
- `loadRawHeightmap()`: `width` or `height` of 1 or above 2048 (valid: 2
    to 2048), `metresPerSample` above 10,000, `seaLevelMetres` beyond
    ±100,000.
- `loadDemFromArrayBuffer()`: GeoTIFFs smaller than 2 or larger than 2048
    samples a side (`DEM_FORMAT_UNSUPPORTED`), strips whose byte counts add
    up to more than the file, and a `verticalScale` that is not finite.
- `setTreeModel()`: more than 393,216 indices, or a position further than
    1000 m from the base.
- `setTreeInstances()`: `tint` or `dryness` outside 0 to 1, or a
    position beyond ±10,000,000 m.
- Error messages from option validation now also give the value that was
    passed.
- The bench speed page builds its table with DOM APIs instead of
  `insertAdjacentHTML`, so no label or error message is parsed as HTML.
- `fixed-scene.mjs` exits non-zero, listing the valid names, when
  `--scene=` names no scene; it printed nothing and succeeded before.
- Altocumulus cloudlets are at least about 110 m across or absent: a
    sparse mackerel sky shows no lone specks.
- The watery sun's corona, and the share of the disc that thick low
    cloud hides, now look through the veil along each pixel's own ray
    rather than taking its mean from the CPU, so they follow the veil's
    fibres and gaps as the disc itself already did. `FrameUniforms.alto.w`
    is no longer used.
- **Grass reaches further.** With the default options, the meadow is
    fully covered (at least 70 % of the ground at density 0.5) to at
    least 60 m at density 0.5 and 40 m at density 4, within the grass
    budgets. The streamed tiles' slots are sized for what thinning and
    the handover keep at each distance, in eight classes out to the view
    distance, and their tufts are counted where the textures put them
    rather than lumped into the tile under each texel's centre; together
    they now fit `GrassOptions.maxInstances`, which they could overrun
    before. Where they would not fit, the full-density radius comes in
    (to 10 m at least) and the widened tufts beyond it keep the cover.
- **No grass ring.** Tufts hand over to the ground's grass sheen from 1.5
    times the full-density radius (at least 0.3 times the grass view
    distance, 66 m by default), and the ground takes exactly the share
    they give up, weighted by how slanted the view is, with the tufts'
    own mean colour and light. The grazing sheen of sunlit blade sides is
    replaced by the tufts' own back-lit glow. From any camera height the
    brightness no longer steps where the tufts end.
- **Terrains are at most 2048 samples a side**, generated, loaded from a
  raw heightmap or GeoTIFF, imported from an image, or read from a
  bundle. The limit is set by WebAssembly's memory, not by a flaw in
  VistaWASM: a WASM module can address at most 4 GiB, a terrain's
  heights, normals, surface, drainage, rivers and vegetation must all
  fit in it, and that memory is never given back while the engine
  lives. A 2048 x 2048 terrain peaks at 0.5 to 1.4 GiB; a 4096 x 4096
  one at up to 3.7 GiB, too close to the limit to rebuild and export it
  safely. A larger map is refused at once with `OPTIONS_INVALID` (or
  `INVALID_DEM` for an image or bundle), before anything is allocated.
  The new exports `MAX_RAW_HEIGHTMAP_BYTES` (16 MiB), `MAX_DEM_BYTES`
  (80 MiB) and `MAX_TERRAIN_SIDE` (2048) give these limits. Painted
  maps, mask images and exports are held to the same 2048: they are
  resampled to or from the terrain, so detail past it would be lost, and
  they are held in WASM memory beside it. Texture images keep 8192.
- **`RiverOptions.riparian`** now also scales the riparian plant band: 0
  removes it and 2 doubles its density.
- **Beds and wet margins** follow the drawn water's edge through a
  channel distance field at four times the heightmap's resolution,
  instead of the heightmap's samples.
- **River paths on every map.** Water is routed by least transversal
  deviation and across flats towards their way out, and each river is
  drawn and carved along a smooth curve that follows its valley, so
  rivers no longer run in straight lines and 45° or 90° zigzags.
  Tributaries meet their main stem at acute angles pointing downstream;
  steep bedrock reaches are narrower; estuaries flare towards the sea;
  bends are carved deeper and steeper on the outside, with a point bar
  inside. Saved seeds make different rivers.
- **`RiverOptions.meanders`** now sets how strongly wide rivers migrate
  by bank erosion; narrower streams keep their small drawn loops.
- **Flowing water.** The current is fastest along the outer bank of
  bends, streaks and standing waves follow the channel (as downstream
  chevrons on rapids), and the flow-map cross-fade no longer makes whole
  rivers blink or lose contrast.
- **Whitewater.** Rivers on generated maps are no longer white from bank
  to bank. Steep channels are rougher, so they run at realistic speeds
  instead of the 6 m/s ceiling. Whitewater covers a share of the surface
  that grows with slope, in clumps on the wave crests, and steep streams
  drop down step-pools: white below each lip, dark and glassy beneath.
  Fast ripples stretch into flow lines, and standing waves, stones and
  their foam fade out before they would flicker.
- **No hanging water.** Steep streams no longer float above the slopes
  they run down. Their levels came from the samples they were routed
  through, but their drawn line runs between them, and across a steep
  face that ground can be metres lower: water hung up to 28 m in the
  air. Drawn water now sits at most its depth and 0.5 m above the
  ground under it.
- **Joins.** A tributary keeps its own width right to its join. Its last
  point, on the main stem, used to take the main stem's discharge, so
  small brooks swelled into wide, triangular funnels over their last few
  hundred metres.
- **Wet banks** are wet brown earth rather than a near-black outline.
- **Small streams.** Mountain rivulets run between soil banks instead of
  rock-walled trenches (rock walls now need over 1 m³/s), and a rill
  running alongside a larger stream on a steep slope is no longer drawn
  as a parallel channel. Their bank strips come and go in patches with
  grass to the water, fade out by 150 m, and their edges wander, so
  streams no longer read as ruled lines across a hillside. Steep streams
  no longer run straight: they wander in long bends across their
  trench and in short ones a few widths long.
- The water vertex grows from 48 to 56 bytes, and `FrameUniforms` from
  960 to 976 bytes, for the eddies.
- **Unknown option keys** are taken out with a `"warning"` that names
  the closest valid key, instead of being ignored silently. They become
  errors in the next major version. Options are now read once, from a
  structured clone, so functions in an options object are refused.
- **Raw heightmaps** are read from the caller's buffer a megabyte at a
  time instead of copied into WASM memory whole. A buffer shorter than
  its options need, or longer than 16 MiB, is refused before anything
  is allocated; one merely longer than needed is read in part with a
  warning (an error in the next major version).
- **Vegetation on vast maps.** Trees, grass, reeds and boulders are
  placed over at most 10,000 km² (a 2048 map at 49 m). A larger
  terrain gets none, with a warning: a few samples kilometres apart
  used to stall the engine for minutes and take gigabytes.
- **Download size.** The WASM is 532,241 bytes gzipped and the
  JavaScript glue 13,583: shader names are shortened at build time,
  wasm-bindgen's hashed import names are shortened after it, and
  `wasm-opt` folds more. Served with Brotli the WASM is 433 KB; the
  [deployment notes](docs/getting-started.md#7-deploy) say how.
  `scripts/check-size.mjs` holds every release file to a size budget and
  prints its Brotli size too.
- **Less per-frame work.** The `start()` loop builds its `RenderStats`
  only while something listens for `"stats"`, a frame allocates nothing
  in the engine once warmed up, and surface weather reuses its buffers
  instead of making new ones every 0.25 s.
- The river build bakes terrain shading once and carves without zeroing
  a map-sized buffer per channel belt; river ribbons leave out the
  vertices inside falls, which nothing drew.
- The engine asks the GPU for the largest buffers it offers. GPU erosion
  of a 2048 map needs 64 MiB buffers, within WebGPU's defaults; were a
  job to need more than the GPU offers, erosion would run on the CPU
  with a warning saying why.

### Fixed

- **Dense forest broke 60 FPS at full resolution.** A jungle at density
  4 cost about 33 ms a frame at 1080p on a mid-range GPU, and only held
  60 FPS once dynamic resolution had halved the resolution. It now costs
  about 12 ms at full resolution: leaves take their light and shadows
  from their cards' corners, and bark and impostors their sky light, so
  a crown many cards deep is not lit again for every layer; understorey
  saplings beyond 15 m are impostor cards; tree shadow casters keep to a
  quarter of the triangle budget; and the triangle budget holds heavy
  meshes to it.
- **Small streams read as white wires** at a distance. A channel now
  hides the low sky from its water, as its banks do, so a small stream
  seen from afar mirrors its dark banks, and its glint and foam shrink
  with its width in pixels.
- **Combs at the snow line.** Distant terrain mesh vertices, several
  samples apart, took the materials of the one sample under them. Where
  snow lies in patches below the snow line, that sampling aliased, and
  the long, thin triangles of the outer bands drew it out into regular
  stripes down the slope. Those vertices now average the materials over
  their own span, as their normals already did, so the snow's edge
  follows the ground softly at every distance.
- **A dry gravel strip at an inflow.** The steep, fast reach where an
  inflow enters at an open map edge was white with foam from bank to
  bank, which at a distance read as a dry gravel bed. Whitewater now
  breaks on the standing waves' crests, with dark, fast water between.
- **Floating and buried trees.** Trees stood at the height of the sample
  under them, not at their own position, and the distant terrain mesh is
  coarser than the heights, so most distant trees floated or sank by
  metres. Trees and grass tufts now stand on the terrain mesh as it is
  drawn, in every level of detail, grounded by a GPU pass each time the
  mesh recentres. On slopes, trees sink to their downhill root point, so
  no side of the trunk floats.
- **Ground by water was too dark.** Bankside moisture no longer turns
  turf to leaf litter and mud or darkens it: riparian ground is greener
  and fresher, and only a thin margin within 2 m of the water is dark and
  wet. Small streams on coarse maps lost their sample-wide brown band:
  the wet-bank distance is measured from their drawn edge.
- **The world ended in a wall.** Past the map edge there was a vertical
  cliff, then a flat, pale sheet of shore and foam instead of sea, and
  smeared streaks along the border. Every terrain now continues as a
  skirt that falls from its edge into deep sea over 1.5 km, with its own
  rock and sand under any snow lying at the edge; mesh vertices beyond
  the edge keep their true places
  instead of collapsing onto the border, and the water takes its depth
  from the skirt.
- **Snow streaked down distant slopes.** Mesh vertices further apart than
  the height samples took the normal of a single sample, which aliased
  and was smeared along the long triangles of the outer mesh bands. Their
  normals now average the slope over each vertex's own spacing. Snow
  also turns from top-down to triplanar texturing smoothly as slopes
  steepen, with no seam, and distant rock, ice and steep snow keep
  triplanar texturing instead of a single top-down sample.
- **Raindrops on the lens were cut off** along straight edges. Each pixel
  only looked at the drop of its own screen cell, so drops that overhung
  their cell were sliced flat. Drops are now simulated on the CPU and
  binned into every screen tile they reach, so they are always whole.
- **Terrain hitches while moving.** The camera-centred terrain mesh was
  rebuilt, reallocated, and re-uploaded (16 MB) in a single frame whenever
  the camera drifted 12 samples. It now streams: the next mesh starts at
  6 samples, centred ahead of the camera by its velocity, is built and
  uploaded 64 rows per frame into a second buffer, and swaps in when
  complete. In testing, a full rebuild took 147 ms of CPU time; while
  streaming, no frame took more than 2 ms.

- **Rain, storms, and snow.** Under rain the sky is now a true overcast
  (brightest overhead, darker at the horizon, no direct sun), so the
  distant sea darkens under rain instead of glowing white, and sharp
  shadows and sun glints on the water disappear. The circular pulse in
  the sky above the camera is gone. Falling rain and snow were smeared
  more the longer a scene ran; rain now falls in short streaks and snow in
  round, chunky flakes. Storms, and `precipitationScale` above 1, now look
  heavier, with a grey veil that cuts visibility. Rain and snow were hidden
  wherever water was in view; they now fall in front of it. Rain curtains
  cost less.
- **Cirrus** drifted at twice the speed of the low clouds, which the
  weather raises in a gale. It now has its own `CloudsOptions.cirrusSpeed`.

- **Lag with a high frame-rate reading.** The demo and examples showed
  `1000 / frameTimeMs` as FPS, but `frameTimeMs` only covers the CPU time
  to submit a frame, so a slow GPU showed tens of thousands of FPS. They
  now show the time between drawn frames. The engine also paces itself:
  `renderOnce()` draws nothing while two earlier frames are still on the
  GPU, so frames can no longer queue up behind a slow GPU and make the
  picture lag behind the camera.
- **Lost GPU devices went unnoticed.** Rendering carried on silently into a
  lost device. The engine now reports `WEBGPU_DEVICE_LOST` on the next
  frame, `start()` emits `"deviceLost"`, and the demo says so.
- GPU erosion falling back to the CPU now adds a warning to the terrain
  metadata, and `generateFractal()` emits `"warning"` events like the
  loaders.

- `npm run dev` and every `npm run dev:*` script failed in a fresh clone
  with "Failed to resolve import \"@vista-wasm/vista-wasm\"" until
  `npm run build` had been run. They now build first when `dist/` is
  missing, explain what to install if the build fails, and warn when the
  build is older than the Rust or TypeScript sources.

- Documentation checked against the code. Corrected: `RenderQualityOptions.preset`
  never capped erosion (that is `ErosionOptions.quality`);
  `RenderStats.frameTimeMs` covers CPU time only; the `"stats"` event also
  fires for `renderOnce()` loops; the fly camera's `←`/`→` keys turn; the
  default `RiverOptions.minCatchmentKm2` is `0.15`; `toVistaWasmError()` and
  `assertVistaWasmSupport()` are not exported; `GPU_LIMIT_EXCEEDED` is not
  raised. The framework components now render at the canvas size and clean
  up when unmounted mid-load, the README and getting-started cameras no
  longer start inside a hill, and every code example type-checks.

- The `"height"`, `"slope"`, `"normals"`, and `"materials"` debug views
  now render.
- The sun disc no longer shines through thick cloud or an overcast sky.
- Cloud edges no longer look hairy: the march bisects to each cloud's
  edge and takes short steps just inside it.
- The README gave the licence as AGPL-3.0-or-later; it is
  AGPL-3.0-only, as `LICENSE`, `NOTICE`, and the package metadata say.
- The declared minimum Rust version (1.82) was too old to build the
  project.
- A failed WASM load is no longer cached, so calling
    `initialiseVistaWasm()` again (for example with a corrected
    `wasmUrl`) tries again.
- `createVistaEngine()` throws a `TypeError` for options that are not an
    object, instead of an `INTERNAL_ERROR`.
- Decoding a PNG, and inflating a bundle's files, no longer copy the data
    through `Blob`s, which makes them several times faster.
- `npm pack` left out `dist/pkg`, the WASM and its JavaScript glue,
    because of the `.gitignore` wasm-pack writes there. The build now
    removes it, and the published package no longer ships declaration maps
    that point at unpublished sources.
- **Painted rivers are channels.** A river painted with a wide brush
  (as the demo's river brush paints one) was thinned into dozens of
  short pieces, each carved on its own at the painted width and only
  0.6 m deep, so it rendered as a blotchy, pale smear. Each stroke now
  becomes one river: the spurs thinning leaves are dropped, the
  centreline is traced end to end, and a stroke ending on another
  painted river flows into it as a tributary. A painted river is as
  deep, as fast and the same colour as a natural river of its width (a
  60 m river is about 4 m deep).
- **Painted water by the sea.** A painted lake's basin could dip below
  the sea, where the sea showed through it as a dark disc with a hard
  edge. A lake's bed now stays at least 0.5 m above the sea, and painted
  water whose rim is less than 1.5 m above the sea joins the sea: it
  shelves out from the land at 1 in 50 and is never deeper than the sea
  floor beside it, so there is no edge and no darker patch.
- **Demo: the noise brush is cheaper.** A dab costs about half what it
  did: it reuses the noise grid points the previous dab computed,
  interpolates the grid row by row with no calls or allocations per
  sample, and weighs only the samples inside its circle. It looks the
  same (heights agree within 0.00001 m), and its brush work a frame
  keeps near 6 ms on a slow phone instead of 9 ms.
- Streamed grass, trees and boulders are no longer handed out before
    their generator can run. The first frames' tiles, the ones round the
    camera, were recorded as filled while the pipelines were still being
    built, and stayed empty until the camera moved away: bare ground
    round a camera close to the ground.
- `npm run build` no longer compiles `wasm-bindgen-cli` from source on
    Apple Silicon Macs, which took minutes and printed
    future-incompatibility warnings about `buf_redux` and `multipart`, and
    no longer reports a missing licence file. `scripts/build-wasm.mjs`
    fetches the prebuilt `wasm-bindgen` that matches `Cargo.lock`, and
    wasm-pack runs with `--no-pack`, since the published package is the
    repository root.
- **Bundled apps ship the WASM.** Vite (and other bundlers) copied the
  WASM glue as a plain file without the `.wasm` beside it, so a
  production build failed to load with `WASM_LOAD_FAILED`. The loader
  now names the WASM's URL itself, where bundlers see it and emit the
  file, and passes `wasmUrl` to the glue in the form it expects, without
  a deprecation warning in the console.
- **Rebuilds no longer double the memory.** Setting a painted map, or
  changing water or biome options, rebuilt the world while still holding
  the old one. The old world is now freed first, on a rebuild and when a
  new terrain replaces the old one: a 2048 x 2048 load, rebuild and
  full-size export of every map peak at 1,364 MiB instead of 1,908 MiB.
- A terrain call or bundle that fails now leaves the engine ready, with
  the terrain, options and painted maps it had. Before, a failed load
  left the engine stuck loading, and a bundle whose heights failed to
  load had already applied its settings.
- After an internal error (a WebAssembly trap), the engine is marked
  dead: `"fatalError"` fires once, an async call the trap left
  unsettled rejects, and every later call throws `ENGINE_DISPOSED`
  instead of reaching the broken instance.
- A lake could drain into a cell at its exact spill level that led back
  into it, so its water ran in a circle. Every walk down the drainage
  graph is now bounded, stops with an error on a cycle, and is checked
  in debug builds.
- Springs on a map hundreds of metres a sample allocated a bucket for
  every 300 m of its area (583 MB on one map), and the buckets that index
  water sounds and river segments could overflow their cell counts on
  a vast map.
- A map under 50 m across drew rivers hanging up to a hundred metres
  above its ground; it now gets none.
- Flora, grass, cloud and mist options without `seedOffset` failed,
  because the wrapper passed it on as `undefined`.
- Frame times come from `performance.now()`, which never jumps with the
  system clock, and float sorts are total, so a `NaN` cannot reorder
  them.

### Security

- The PNG decoder and the bundle reader allocate their output only once
    the compressed data has proved to be as long as its header says, so a
    small file whose header claims a huge image or file cannot claim the
    memory.
- A bundle's `manifest.json` is checked against its 1 MiB limit before it
    is inflated.
- Options passed from JavaScript cannot change object prototypes, and
    CSV exports hold only numbers and species names, so no cell can start
    a formula.
- Loaded heightmaps and GeoTIFFs are checked against their size limits
    with overflow-free arithmetic before anything is allocated. Before, a
    GeoTIFF header could make the decoder reserve memory for any size it
    claimed, and on 32-bit WebAssembly a raw heightmap's size could
    overflow and read past its buffer, stopping the engine.
- Every offset and count in a GeoTIFF is bounds checked without
    overflow.
- Heightmap and GeoTIFF samples that are not finite, or lie beyond
    ±100 km once scaled, become no data with a warning instead of spreading
    through the terrain.
- Grass view, tree mesh and vegetation detail distances are capped, so
    a large value can no longer stall the page laying out vegetation tiles.
- `setSun()`, `setAtmosphere()`, `setClouds()`, `setMist()`,
    `setWeather()`, `setShadows()`, `setWater()` and `setBiomes()` validate
    in the engine itself, so no path skips validation.
- Tree placements are counted before they are unpacked, and
    `getWaterSounds()` returns nothing for positions that are not finite.
- Noise and sound-grid lookups far from the terrain no longer overflow.
- **The demo and the vanilla example set a Content Security Policy**
  in a `<meta http-equiv>` tag: `default-src 'self'`, `script-src 'self'
  'wasm-unsafe-eval'` (plus the hash of the demo's inline import map),
  `object-src 'none'`, `base-uri 'none'` and `form-action 'none'`. The
  demo's biome legend moved its colours from style attributes into its
  stylesheet so no inline style is needed. `npm run build:demo` checks the
  import map's hash. The vanilla example's policy goes into its
  production build only, as Vite's dev server injects inline styles.
- **Demo file checks.** Every file input in the demo (Explore's imports
  and texture replacement, and the Paint tab) checks a file's type and
  size, and an image's pixel size from its header, before anything
  decodes it: PNG, JPEG or WebP images of at most 160 MB, 2 to 2048
  pixels a side for heightmaps and painted maps and 2 to 8192 for
  textures, and `.zip` bundles of at most 512 MB, with a message
  that says what to choose instead.
- Inputs that could exhaust memory are refused before anything is
  allocated for them: raw heightmaps and GeoTIFFs past their largest
  size, DEM downloads past `maxBytes` (cancelled mid-stream),
  heightmap images, painted maps and bundles past 2048 a side (read
  from their headers before anything decodes them), other images past
  8192, bundles whose painted maps would decode past 1 GiB together, and
  more than 1,000,000 custom trees.
- The build checks the prebuilt `wasm-bindgen` it downloads against
  recorded SHA-256 checksums and refuses any other archive.
- Every push and pull request runs Clippy with no warnings allowed, the
  tests and fuzzers (now including a fuzzer over the terrain and river
  pipeline), `cargo audit`, `npm audit`, a size check and a browser
  smoke test, and the demo deploys only when they pass.

### Removed

- `shaders/flora_instances.wgsl`, replaced by `shaders/trees.wgsl`.
- `shaders/terrain_noise.wgsl`, which nothing used.
- The unused `shaders/material_masks.wgsl` placeholder.
- `shaders/shadow.wgsl`: tree shadows are drawn by `shaders/trees.wgsl`,
  so they sway with the trees.
- The hand-built tree meshes, replaced by grown ones.

### Upgrading from 1.0.0

No code changes are needed; every new option is optional. Some defaults
changed, so scenes look (and cost) different:

- `FloraOptions.treeQuality` defaults to `"mesh"`, `speciesVariation` to
  `0.6`, and `windStrength` to `0.3`. `"billboard"` and `"cross-quad"`
  draw impostors of the real tree models.
- `CloudsOptions.heightMetres` defaults to `1800` and `raymarchSteps` to
  `32`. Clouds render at half resolution (`resolutionScale: 0.5`) and add
  a thin cirrus layer (`cirrus: 0.35`); set `cirrus: 0` to remove it.
- Terrain, tree, and cloud shadows are on by default. Pass
  `shadows: { terrain: { enabled: false }, trees: { enabled: false } }`
  for the 1.0.0 look, or lower `trees.resolution` on weak GPUs.
- Grass is on by default, and its `density` now describes cover: 0.5 is
  a natural meadow, far denser than before. Pass `enabled: false` to
  turn it off, or a lower density for sparser grass.
- The weather system is off by default, so existing cloud, mist, and water
  settings behave as before until you enable it.
- Fractal terrain comes from the new generator, so saved seeds give new
  maps. Erosion iteration counts from 1.0.0 still work but do much less at
  the old values; leave them unset and pick a `quality` instead.

Some inputs 1.0.0 accepted are now refused at once, with a message
saying why:

- Terrains, heightmap images, painted maps, masks and exports over 2048
  samples a side. WebAssembly can address 4 GiB, and a larger terrain,
  with its rivers, biomes and vegetation, leaves too little of it to
  rebuild and export safely. Resample larger maps to 2048 a side first,
  and raise `metresPerSample` to match, so the ground keeps its size.
- Raw heightmaps over 16 MiB, GeoTIFFs over 80 MiB, and DEM downloads
  past `maxBytes`.
- Functions in an options object. Unknown option keys are still
  accepted, with a `"warning"` naming the closest valid key; they become
  errors in the next major version.

## [1.0.0] — 2026-07-10

### Added

- WebGPU terrain engine compiled to WebAssembly via Rust and wasm-bindgen.
- Seeded fractal terrain generation with configurable noise type (simplex,
  ridged), octaves, gain, lacunarity, and shaping passes (island, terrace,
  basin, canyon, crater).
- Hydraulic and thermal erosion. Runs as GPU compute passes in the browser
  (falling back to the CPU implementation automatically); runs CPU-side on
  native/test builds.
- GeoTIFF DEM import: uncompressed strips, 16-bit integer, 32-bit integer,
  and 32-bit float samples; ModelPixelScaleTag, ModelTiepointTag, and
  GDAL no-data values.
- Raw heightmap loading (float32, uint16, int16, uint8, int8 sample formats).
- Single-mesh terrain renderer with exponentially increasing sample spacing
  from the camera, giving near-detail plus far reach without the T-junction
  cracks that multi-tier clipmaps need skirts to hide. Mesh recentres on the
  camera as it moves.
- Analytic Rayleigh/Mie sky dome with sun disc/glare, horizon haze, and an
  optional cloud layer (off/painted/volumetric).
- Height-based ground mist (off/flat/volumetric) applied across terrain,
  flora, grass, and water.
- Tree billboard renderer with billboard/cross-quad/mesh quality dial,
  per-tree canopy variety, and wind sway. Placement respects slope, tree
  line, water level, and flora density.
- Grass ground-cover renderer: crossed blade tufts driven by terrain material
  weights, with configurable view-distance fade.
- Animated fresnel/specular water plane, sized to the terrain footprint.
- Fly-camera controller: WASD movement, pointer-drag look, scroll-to-zoom,
  Space/Shift and middle-mouse-drag for vertical travel.
- Terrain export: hypsometric minimap PNG (`exportHeightmapImage`), Wavefront
  OBJ 3D model (`exportTerrainObj`), raw heightmap download, and canvas
  screenshot (`exportSnapshot`).
- TypeScript wrapper (`js/src`) with full types and a pending-call mutex that
  prevents wasm-bindgen reentrancy panics when async calls overlap with the
  render/resize loop.
- Demo as a plain static site (no bundler at request time) deployable to
  GitHub Pages via `.github/workflows/deploy-demo.yml`.
- Four runnable framework examples: vanilla TypeScript, React, Vue, Svelte.
- Rust workspace with separate `vista_types` and `vista_wasm` crates.
- Comprehensive documentation in `docs/` covering every public option,
  terrain data, vegetation, sky and weather, water, camera and controls,
  render quality, export, events and errors, architecture, and contributing.

[2.0.0]: https://github.com/0xe25f/vistawasm/compare/v1.0.0...v2.0.0
[1.0.0]: https://github.com/0xe25f/vistawasm/releases/tag/v1.0.0
