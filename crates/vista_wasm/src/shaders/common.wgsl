// Shared prelude prepended to every render shader (see `render/gpu.rs`,
// which concatenates this file in front of each shader at pipeline
// creation). Keeping the frame uniforms, world textures, sky model,
// lighting, and fog maths in one place guarantees that terrain, trees,
// grass, water, sky, and clouds all agree on colour and lighting.
//
// All shading happens in linear light. `finish_colour` applies exposure,
// ACES filmic tone mapping, and (when the canvas format is not an sRGB
// format) the sRGB transfer curve.

struct FrameUniforms {
  view_proj: mat4x4<f32>,
  // xyz: camera position, w: animation time in seconds.
  camera_position: vec4<f32>,
  // xyz: forward, w: tan(fov_y / 2).
  camera_forward: vec4<f32>,
  // xyz: right, w: aspect ratio.
  camera_right: vec4<f32>,
  // xyz: up, w: 1 when the shader must apply the sRGB curve itself.
  camera_up: vec4<f32>,
  // xyz: direction towards the sun, w: sun intensity.
  sun_direction: vec4<f32>,
  // x: Rayleigh strength, y: Mie strength, z: haze distance, w: exposure.
  atmosphere: vec4<f32>,
  // rgb: sky tint, w: debug view index.
  sky_tint: vec4<f32>,
  // x: density, y: base height, z: height falloff, w: noise strength.
  mist_params: vec4<f32>,
  // rgb: mist colour, w: water level for the rise-above-water term.
  mist_colour: vec4<f32>,
  // xy: wind offset in metres, z: sun scattering, w: seed phase.
  mist_wind: vec4<f32>,
  // x: coverage, y: base height, z: thickness, w: raymarch steps (0 = painted).
  cloud_params: vec4<f32>,
  // xy: wind offset in metres, z: evolution offset, w: density.
  cloud_motion: vec4<f32>,
  // rgb: cloud colour, w: 1 when clouds cast shadows.
  cloud_colour: vec4<f32>,
  // x: ripple scale, y: reflectivity, z: clarity (metres), w: foam.
  water_params: vec4<f32>,
  // rgb: shallow colour, w: sea level.
  water_shallow: vec4<f32>,
  // rgb: deep colour, w: near plane.
  water_deep: vec4<f32>,
  // xy: current offset in metres, z: current speed, w: far plane.
  water_current: vec4<f32>,
  // x: amplitude, y: wavelength, z: direction (radians), w: steepness.
  wave_params: vec4<f32>,
  // x: speed, y: directional spread, z: waves enabled, w: unused.
  wave_params2: vec4<f32>,
  // xy: snapped ocean grid origin, z: 1 when the ocean is drawn, w: unused.
  water_origin: vec4<f32>,
  // x: wind, y: variation, z: tree style, w: grass view distance.
  vegetation: vec4<f32>,
  // x: tree mesh distance, y: the share of a meadow its tufts cover near
  // the camera (0 without grass), z: grass full-density radius, w:
  // meadow grass height.
  vegetation2: vec4<f32>,
  // xy: viewport size in pixels, zw: reciprocal.
  viewport: vec4<f32>,
  // Light-space transform for the tree shadow map.
  shadow_view_proj: mat4x4<f32>,
  // x: tree shadow strength (0 = off), y: filter radius in texels,
  // z: terrain shadow strength (0 = off), w: cloud shadow strength.
  shadow_params: vec4<f32>,
  // x: rain, y: snowfall, z: ground wetness, w: settled snow.
  weather: vec4<f32>,
  // x: lightning flash, y: overcast greyness, zw: wind (m/s) for rain.
  weather2: vec4<f32>,
  // x: textures on, y: detail normals on, z: texture scale, w: unused.
  surface: vec4<f32>,
  // x: cirrus amount (0 = none), y: cirrus altitude, zw: unit wind
  // direction (x, z) that stretches cirrus into streaks.
  clouds2: vec4<f32>,
  // x: stratiform (0 cumulus, 1 sheet), y: towering storm clouds,
  // z: base darkness, w: ragged base.
  clouds3: vec4<f32>,
  // x: rain shafts, yz: latest lightning strike (x, z), w: how many times
  // taller than the ordinary cloud layer the slab is stretched for towers.
  clouds4: vec4<f32>,
  // xy: cirrus wind offset in metres, z: 1 while drops are on the lens,
  // w: precipitation heaviness (1 = full rain or snow, more = downpour).
  weather3: vec4<f32>,
  // The previous frame's view-projection, for reusing its clouds.
  previous_view_proj: mat4x4<f32>,
  // x: 1 when sky cloud pixels may be reused from the previous frame,
  // y: which pixel of each 2 x 2 block is raymarched this frame (0 to 3),
  // zw: size of the cloud image in pixels.
  temporal: vec4<f32>,
  // x: render distance, y: terrain detail distance, z: cloud distance
  // (metres; 1e9 means unlimited), w: unused.
  distances: vec4<f32>,
  // x: render distance fade length, y: cloud fade length (metres), zw:
  // unused.
  fades: vec4<f32>,
  // xy: canvas size in pixels, z: render scale (the scene is rendered at
  // `viewport` = canvas x scale), w: unused.
  output: vec4<f32>,
  // x: blowing snow (0 to 1), yzw: unused.
  cold: vec4<f32>,
  // x: 1 when any sea may freeze, y: temperature unit of the open sea
  // beyond the terrain, zw: wind drift of the floes in metres.
  sea_ice: vec4<f32>,
  // x: snowmelt fullness of the rivers (0.4 to 1.4), y: 1 when any lake,
  // river or waterfall is below 0 °C, z: 1 when there are waterfalls,
  // w: 1 when the wet-bank field is loaded.
  rivers: vec4<f32>,
  // The terrain mesh being drawn, for `mesh_surface_height`: xy its centre
  // sample, z metres per sample (0 when none is drawn), w how far it
  // builds the skirt, in samples.
  ground: vec4<f32>,
  // x: crown area per square metre per unit of cover share (0 without
  // trees), y: canopy distance, z: tree shadow distance (metres), w: 1
  // when the canopy layer is drawn.
  vegetation3: vec4<f32>,
  // Bare rock (`materials.wgsl`): xy the beds' rise per metre along x and z,
  // z their spacing in metres, w the angle of the sunward side in
  // radians.
  rock: vec4<f32>,
  // The regional weather map (`regional_weather_at`): xy its corner in
  // metres, z 1 / its size, w 1 when clouds and rain read it.
  regional: vec4<f32>,
  // Aerosols: rgb the Mie colour (bluer in dry air), w the Mie phase
  // asymmetry (larger in damp air). (1, 1, 1, 0.76) without weather.
  air: vec4<f32>,
  // Light under cloud: x direct sun, y shadow strength (applied on the
  // CPU), z indirect light, w how far sky light is the flat light of a
  // cloud deck. (1, 1, 1, 0) without weather.
  light: vec4<f32>,
  // The gust front (`gust_front`): x the distance it has travelled
  // downwind in metres, y gustiness, z mean wind in m/s, w 1 when on.
  gust: vec4<f32>,
  // x: 1 when the surface weather map is read, y: whitecap coverage (0
  // to 0.3), z: blown spray (0 to 1), w: unused.
  weather4: vec4<f32>,
  // x: variation of low-cloud base height (a share of the layer's
  // thickness), y: lumps under low clouds (0 to 1), z: altocumulus, w:
  // altostratus (0 = none).
  clouds5: vec4<f32>,
  // The mid-level layer: xy its drift in metres, z its height in metres,
  // w unused.
  alto: vec4<f32>,
  // x: eddies and vortices in rivers (0 to 1; 0 skips them), y: 1 when
  // boulder meshes draw the stream stones, zw: the stream stones' lattice
  // seed, its low and high 16 bits.
  rivers2: vec4<f32>,
  // Riparian scrub's thinning with distance, as the tree cull thins it:
  // x the full-density radius (0 when no trees are drawn), y the share
  // kept at any distance, z the distance by which every tree has given
  // way to the canopy layer. w: refraction and caustics in shallow water
  // (0 to 1; 0 skips them).
  waterside: vec4<f32>,
  // The river mouths whose plumes tint the sea (`water::plume_mouths`),
  // two each: x, z and the way out to sea; then width (0 for none), plume
  // length and the catchment's colour.
  mouths: array<vec4<f32>, 16>,
};

struct WorldInfo {
  // Per species: x height, y radius (metres), z trunk bend and w leaf
  // flutter in the wind.
  species: array<vec4<f32>, 8>,
  // Per species: rgb foliage tint, w unused.
  species_tint: array<vec4<f32>, 8>,
  // xy: terrain half extents (metres), zw: metres per height texel.
  terrain: vec4<f32>,
  // xy: height texture size, z: 1 when a terrain is loaded, w: the
  // terrain's sea level.
  terrain2: vec4<f32>,
  // Per material colour multiplier (rgb).
  material_tints: array<vec4<f32>, 12>,
  // x: variants per species in the impostor atlas.
  trees: vec4<f32>,
  // Per species: the canopy layer's colour (rgb), averaged from its
  // impostors.
  species_canopy: array<vec4<f32>, 8>,
};

@group(0) @binding(0) var<uniform> frame: FrameUniforms;

@group(1) @binding(0) var linear_sampler: sampler;
@group(1) @binding(1) var terrain_albedo: texture_2d_array<f32>;
@group(1) @binding(2) var terrain_normal: texture_2d_array<f32>;
@group(1) @binding(3) var flora_texture: texture_2d_array<f32>;
@group(1) @binding(4) var impostor_texture: texture_2d_array<f32>;
@group(1) @binding(5) var noise_texture: texture_2d<f32>;
@group(1) @binding(6) var cloud_texture: texture_3d<f32>;
@group(1) @binding(7) var water_texture: texture_2d<f32>;
@group(1) @binding(8) var height_texture: texture_2d<f32>;
@group(1) @binding(9) var<uniform> world: WorldInfo;
@group(1) @binding(10) var terrain_shadow_texture: texture_2d<f32>;
@group(1) @binding(11) var clamp_sampler: sampler;
// Per-terrain data at height texture resolution, one layer each, always
// sampled with `textureSampleLevel`:
// - layer 0, the surface: r temperature unit ((°C + 30) / 65), g
//   moisture, b permanent snow (fast ice on the sea), a biome index / 255;
// - layer 1, the banks: r distance to the nearest river, lake or
//   waterfall edge / 40 m, g snow and ice cover, b bankside greening (0 to
//   1), a the talus field boulders are placed from (`soil::talus_byte`,
//   read as bytes by the generators);
// - layer 2, tree cover (`flora::bake_cover`): r the share of the
//   slider's density the land holds, as sqrt(share / 4); g and b the
//   dominant and second species; a the second species' share out of 127,
//   plus 128 where trees are stunted.
@group(1) @binding(12) var ground_layers: texture_2d_array<f32>;
// The regional weather map (`weather/regional.rs`): r cloud coverage, g
// precipitation, b storminess, a humidity. Read by `regional_weather_at`.
@group(1) @binding(13) var regional_texture: texture_2d<f32>;
// The surface weather map over the terrain (`surface_weather.wgsl`): r
// wetness, g puddle water, b snow depth, a a hollow's fill. Read by
// `surface_weather_at`.
@group(1) @binding(14) var surface_weather_texture: texture_2d<f32>;

// Shadow receivers only (terrain, trees, grass, water).
@group(2) @binding(0) var tree_shadow_map: texture_depth_2d;
@group(2) @binding(1) var shadow_sampler: sampler_comparison;

const PI: f32 = 3.14159265;
const TAU: f32 = 6.2831853;

fn remap(value: f32, old_min: f32, old_max: f32, new_min: f32, new_max: f32) -> f32 {
  return new_min + (value - old_min) / max(old_max - old_min, 0.0001) * (new_max - new_min);
}

fn time_seconds() -> f32 {
  return frame.camera_position.w;
}

fn hash11(value: f32) -> f32 {
  var x = fract(value * 0.1031);
  x = x * (x + 33.33);
  return fract(x * (x + x));
}

// Interleaved gradient noise: a cheap, well-distributed per-pixel dither
// used for alpha-test cross-fades and to hide raymarch banding.
fn pixel_dither(pixel: vec2<f32>) -> f32 {
  return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

fn srgb_to_linear(colour: vec3<f32>) -> vec3<f32> {
  return pow(max(colour, vec3<f32>(0.0)), vec3<f32>(2.2));
}

fn aces(colour: vec3<f32>) -> vec3<f32> {
  let a = colour * (colour * 2.51 + 0.03);
  let b = colour * (colour * 2.43 + 0.59) + 0.14;
  return clamp(a / b, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn finish_colour(linear: vec3<f32>) -> vec3<f32> {
  var mapped = aces(max(linear, vec3<f32>(0.0)) * frame.atmosphere.w);

  if (frame.camera_up.w > 0.5) {
    mapped = pow(mapped, vec3<f32>(1.0 / 2.2));
  }

  return mapped;
}

fn sun_dir() -> vec3<f32> {
  return normalize(frame.sun_direction.xyz);
}

fn henyey_greenstein(cos_theta: f32, g: f32) -> f32 {
  let g2 = g * g;
  return (1.0 - g2) / (4.0 * PI * pow(max(1.0 + g2 - 2.0 * g * cos_theta, 0.0001), 1.5));
}

// --- Sky model -----------------------------------------------------------
//
// A compact single-scattering sky: Rayleigh and Mie optical depths use a
// relative air-mass approximation, sunlight is attenuated through the same
// atmosphere (so it reddens at sunrise and sunset), and in-scattering uses
// the analytic "extinction-weighted" integral. It costs a handful of
// exponentials per call, so terrain, water reflections, and fog can all
// query it directly and stay consistent with the visible sky.

const RAYLEIGH_BETA: vec3<f32> = vec3<f32>(5.8e-6, 13.5e-6, 33.1e-6);
const MIE_BETA: vec3<f32> = vec3<f32>(2.1e-5, 2.1e-5, 2.1e-5);
const RAYLEIGH_HEIGHT: f32 = 8000.0;
const MIE_HEIGHT: f32 = 1200.0;
const SUN_RADIANCE: f32 = 15.0;

fn air_mass(cos_zenith: f32) -> f32 {
  let c = clamp(cos_zenith, -0.2, 1.0);
  let zenith_degrees = degrees(acos(c));
  return 1.0 / max(c + 0.50572 * pow(max(96.07995 - zenith_degrees, 0.05), -1.6364), 0.02);
}

fn rayleigh_beta() -> vec3<f32> {
  return RAYLEIGH_BETA * max(frame.atmosphere.x, 0.0);
}

fn mie_beta() -> vec3<f32> {
  return MIE_BETA * (0.1 + max(frame.atmosphere.y, 0.0) * 0.45) * frame.air.rgb;
}

fn sun_transmittance(cos_zenith: f32) -> vec3<f32> {
  let m = air_mass(cos_zenith);
  let transmittance = exp(-(rayleigh_beta() * RAYLEIGH_HEIGHT + mie_beta() * MIE_HEIGHT) * m);
  // Fade out as the sun sinks below the horizon.
  return transmittance * smoothstep(-0.08, 0.02, cos_zenith);
}

// Sunlight reaching the ground, in the same units as `sky_radiance`.
// Cloud between the camera and the sun, and a full overcast, diffuse most
// direct light into the sky's ambient (`frame.light.x`).
fn sun_light() -> vec3<f32> {
  return sun_transmittance(sun_dir().y) * frame.sun_direction.w * 2.35 * frame.light.x;
}

fn luminance(colour: vec3<f32>) -> f32 {
  return dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn sky_radiance(direction: vec3<f32>) -> vec3<f32> {
  let sun = sun_dir();
  let dir = normalize(vec3<f32>(direction.x, max(direction.y, 0.0), direction.z));
  let mu = dot(dir, sun);
  let br = rayleigh_beta();
  let bm = mie_beta();
  let m = air_mass(dir.y);
  let depth_r = br * RAYLEIGH_HEIGHT * m;
  let depth_m = bm * MIE_HEIGHT * m;
  let extinction = depth_r + depth_m;
  let phase_r = 0.0596831 * (1.0 + mu * mu);
  let phase_m = henyey_greenstein(mu, frame.air.w);
  // Light scattered high in the sky has crossed only part of the air mass
  // the ground-level sun has, so it is less reddened (square root = half
  // the optical depth).
  let sun_colour = sqrt(sun_transmittance(sun.y)) * smoothstep(-0.08, 0.02, sun.y);
  let inscatter = (depth_r * phase_r + depth_m * phase_m) / max(extinction, vec3<f32>(1e-6))
    * (vec3<f32>(1.0) - exp(-extinction));
  // A little extra multiple-scattering fill so twilight skies are not
  // pitch black and deep blue survives near the zenith.
  let fill = vec3<f32>(0.004, 0.009, 0.022) * smoothstep(-0.3, 0.2, sun.y);
  let night = vec3<f32>(0.0012, 0.0018, 0.004);
  var sky = (inscatter * sun_colour * SUN_RADIANCE * frame.sun_direction.w + fill + night)
    * frame.sky_tint.rgb;
  // Weather: an overcast sky is a uniform grey deck, brightest overhead and
  // about a third as bright at the horizon (the CIE overcast sky), so the
  // bright clear-sky horizon must not show through it. Its level follows
  // the clear sky at the zenith, lit by the sun above the deck.
  let overcast = frame.weather2.y;

  if (overcast > 0.001) {
    let zenith_mu = sun.y;
    let zenith_inscatter = (br * RAYLEIGH_HEIGHT * 0.0596831 * (1.0 + zenith_mu * zenith_mu)
      + bm * MIE_HEIGHT * henyey_greenstein(zenith_mu, frame.air.w)) / (br * RAYLEIGH_HEIGHT + bm * MIE_HEIGHT)
      * (vec3<f32>(1.0) - exp(-(br * RAYLEIGH_HEIGHT + bm * MIE_HEIGHT)));
    let zenith = luminance((zenith_inscatter * sun_colour * SUN_RADIANCE * frame.sun_direction.w + fill + night)
      * frame.sky_tint.rgb);
    let deck = zenith * 2.0 * (1.0 + 2.0 * dir.y) / 3.0 * (1.0 - overcast * 0.4);
    sky = mix(sky, vec3<f32>(deck) * vec3<f32>(0.92, 0.95, 1.0), overcast);
  }

  return sky + vec3<f32>(0.75, 0.8, 1.0) * frame.weather2.x * 1.6;
}

// Diffuse sky light arriving on a surface with normal `normal`. Under a
// cloud deck (`frame.light`) it dims, and comes evenly from the cloud
// base lit from above instead of brighter from the sunward horizon.
fn sky_irradiance(normal: vec3<f32>) -> vec3<f32> {
  let up = sky_radiance(vec3<f32>(0.0, 1.0, 0.0));
  let towards_sky = saturate(normal.y * 0.5 + 0.5);

  // Most callers ask about an upward-facing surface, which sees no
  // horizon term, so skip evaluating the sky model a second time.
  if (towards_sky >= 0.9999) {
    return up * 2.4 * frame.light.z;
  }

  let horizon = sky_radiance(normalize(vec3<f32>(-sun_dir().x, 0.2, -sun_dir().z)));
  let sky = mix(mix(horizon, up, towards_sky), up, frame.light.w);
  // Light bounced off the ground below.
  let bounce = sun_light() * 0.035 * saturate(-normal.y * 0.5 + 0.5);
  return (sky * (0.55 + 0.45 * saturate(normal.y * 0.5 + 0.5)) * 2.4 + bounce) * frame.light.z;
}

// --- Terrain height lookups ---------------------------------------------

fn terrain_height_at(xz: vec2<f32>) -> f32 {
  return ground_height(height_texture, world.terrain, world.terrain2, xz);
}

// The ground as the terrain mesh is drawn this frame, at a world
// position (`mesh_height` in `ground.wgsl`).
fn mesh_surface_height(xz: vec2<f32>) -> f32 {
  return mesh_height(height_texture, world.terrain, world.terrain2, frame.ground, xz);
}

// Whether a world position lies over the loaded terrain.
fn over_terrain(xz: vec2<f32>) -> bool {
  return world.terrain2.z > 0.5 && all(abs(xz) <= world.terrain.xy);
}

// A ground layer at a world position (see `ground_layers`).
fn ground_layer_at(xz: vec2<f32>, layer: i32) -> vec4<f32> {
  let size = max(world.terrain2.xy, vec2<f32>(1.0));
  let uv = ((xz + world.terrain.xy) / world.terrain.zw + 0.5) / size;
  return textureSampleLevel(ground_layers, clamp_sampler, uv, layer, 0.0);
}

// The surface layer at a world position (see `ground_layers`).
fn surface_at(xz: vec2<f32>) -> vec4<f32> {
  return ground_layer_at(xz, 0);
}

// Water by a world position: x the distance to the nearest water in
// metres, up to 40 m, y bankside greening (0 to 1).
fn water_banks_at(xz: vec2<f32>) -> vec2<f32> {
  let texel = ground_layer_at(xz, 1);
  return vec2<f32>(texel.r * 40.0, texel.b);
}

// Snow and ice on the ground at a world position, 0 to 1: its materials
// and snow that never melts.
fn snow_cover_at(xz: vec2<f32>) -> f32 {
  return ground_layer_at(xz, 1).g;
}

// The cover layer at a world position (see `ground_layers`).
fn cover_at(xz: vec2<f32>) -> vec4<f32> {
  return ground_layer_at(xz, 2);
}

// The regional weather at a world position: r cloud coverage, g
// precipitation, b storminess, a humidity. Only meaningful while
// `frame.regional.w` is 1.
fn regional_weather_at(xz: vec2<f32>) -> vec4<f32> {
  let uv = (xz - frame.regional.xy) * frame.regional.z;
  return textureSampleLevel(regional_texture, clamp_sampler, uv, 0.0);
}

// The wet and snowy ground at a world position: x wetness, y puddle
// water, z snow depth, w 1 when read from the surface weather map. Off
// the terrain, or while the weather does not drive the ground, the
// weather's single values (0 without weather), with no puddle water.
fn surface_weather_at(xz: vec2<f32>) -> vec4<f32> {
  if (frame.weather4.x < 0.5 || !over_terrain(xz)) {
    return vec4<f32>(frame.weather.z, 0.0, frame.weather.w, 0.0);
  }

  let uv = (xz + world.terrain.xy) / max(world.terrain.xy * 2.0, vec2<f32>(1.0));
  let texel = textureSampleLevel(surface_weather_texture, clamp_sampler, uv, 0.0);
  return vec4<f32>(texel.rgb, 1.0);
}

// Settled snow at a world position, 0 to 1: the weather's, not snow that
// lies all year.
fn settled_snow_at(xz: vec2<f32>) -> f32 {
  return surface_weather_at(xz).z;
}

// The shared gust front (`weather/wind.rs`'s `gust`): -1 in a lull, 1 in
// a gust, sweeping downwind across the land. The distance it has
// travelled comes from the CPU, so nothing here multiplies time.
fn gust_front(xz: vec2<f32>) -> f32 {
  let direction = normalize(frame.weather2.zw + vec2<f32>(0.00001, 0.0));
  let s = dot(xz, direction) - frame.gust.x;
  return gust_noise(s / 220.0) * 0.65 + gust_noise(s / 70.0 + 17.0) * 0.35;
}

fn gust_noise(s: f32) -> f32 {
  let cell = floor(s);
  let t = s - cell;
  let fade = t * t * (3.0 - 2.0 * t);
  return mix(hash11(cell), hash11(cell + 1.0), fade) * 2.0 - 1.0;
}

// Expanding rain rings in a jittered grid of 0.9 m cells, each restarting
// at its own random time: a small normal tilt (xz). Shared by open water
// and puddles.
fn rain_rings(xz: vec2<f32>) -> vec2<f32> {
  let cell_size = 0.9;
  let cell = floor(xz / cell_size);
  let local = xz / cell_size - cell;
  let seed = hash12(cell);
  let centre = vec2<f32>(hash12(cell + 17.1), hash12(cell + 3.7)) * 0.6 + 0.2;
  let age = fract(time_seconds() * (0.8 + seed * 0.6) + seed);
  let offset = local - centre;
  let radius = length(offset);
  let ring = sin((radius - age * 0.5) * 48.0) * (1.0 - age) * smoothstep(0.5, 0.0, radius)
    * step(radius, age * 0.5 + 0.05);
  return offset / max(radius, 0.001) * ring;
}

// The share of the ground under tree crowns at a world position, 0 to 1:
// `lattice::canopy_cover` at this frame's tree density.
fn canopy_at(xz: vec2<f32>) -> f32 {
  if (!over_terrain(xz) || frame.vegetation3.x <= 0.0) {
    return 0.0;
  }

  let red = cover_at(xz).r * 255.0;
  return 1.0 - exp(-frame.vegetation3.x * red * red * (4.0 / 65025.0));
}

// Clumps of tree crowns seen from afar: cellular noise with cells of 6 to
// 12 m, 1 at a crown's centre and 0 in the gaps between crowns. Shared by
// the canopy layer and the dappled ground under it.
fn canopy_clumps(xz: vec2<f32>) -> f32 {
  let p = xz / 9.0;
  let cell = floor(p);
  var nearest = 2.0;

  for (var j = -1; j <= 1; j = j + 1) {
    for (var i = -1; i <= 1; i = i + 1) {
      let c = cell + vec2<f32>(f32(i), f32(j));
      let centre = c + vec2<f32>(hash12(c), hash12(c + 17.31));
      // Crowns from two thirds to four thirds of the cell.
      let size = 0.67 + 0.67 * hash12(c + 41.7);
      nearest = min(nearest, length(p - centre) / size);
    }
  }

  return saturate(1.0 - nearest);
}

// Whether crown clumps at `distance` are smaller than about two pixels,
// where their pattern would only alias.
fn clumps_unresolved(distance: f32) -> bool {
  return distance * frame.camera_forward.w * 2.0 * frame.viewport.w > 4.5;
}

// `canopy_clumps` seen from `distance`: its mean where unresolved.
fn canopy_clumps_at(xz: vec2<f32>, distance: f32) -> f32 {
  if (clumps_unresolved(distance)) {
    return 0.57;
  }

  return canopy_clumps(xz);
}

// Snow that never melts at a world position, 0 to 1.
fn permanent_snow_at(xz: vec2<f32>) -> f32 {
  if (!over_terrain(xz)) {
    return 0.0;
  }

  return surface_at(xz).b;
}

// --- Clouds (shared by the sky pass and cloud shadows) -------------------

fn cloud_weather(xz: vec2<f32>) -> f32 {
  return cloud_weather_level(xz).x;
}

// `cloud_weather`, and in y the level low-cloud bases condense at there (0
// to 1): fractal noise about 1.6 km across, drifting with the weather,
// from a channel of the same lookup, so it costs nothing.
fn cloud_weather_level(xz: vec2<f32>) -> vec2<f32> {
  var coverage = frame.cloud_params.x;

  // Weather that varies across the map sets the coverage locally.
  if (frame.regional.w > 0.5) {
    coverage = regional_weather_at(xz).r;
  }

  let uv = (xz + frame.cloud_motion.xy) / 26000.0;
  let large = textureSampleLevel(noise_texture, linear_sampler, uv, 0.0);
  let medium = textureSampleLevel(noise_texture, linear_sampler, uv * 3.3 + vec2<f32>(0.37, 0.71), 0.0).g;
  let weather = large.r * 0.7 + medium * 0.3;
  return vec2<f32>(saturate(remap(weather, 1.0 - coverage * 0.95 - 0.05, 1.0 - coverage * 0.35, 0.0, 1.0)), large.a);
}

// --- The mid-level layer (shared by the sky pass and cloud shadows) -----
//
// Altocumulus and altostratus share one thin slab at `frame.alto.z`, drawn
// as a single 2D layer, as cirrus is: a view ray meets its mid-plane once.
// Every lookup is a 2D `textureSampleLevel`, so any of them may sit
// behind a per-pixel branch.

// Depth of each kind of alto cloud, in metres.
const ALTOCUMULUS_THICKNESS: f32 = 250.0;
const ALTOSTRATUS_THICKNESS: f32 = 600.0;
// Extinction per metre at density 1: altostratus at amount 1 (a mean
// density of 0.85) lets through about 8 % of the light overhead.
const ALTO_EXTINCTION: f32 = 0.00495;
// Spacing of the altocumulus ripples, in metres.
const ALTO_RIPPLE: f32 = 900.0;
// One tile of the Worley channel holds six cells of about 180 m.
const ALTO_CELL_TILE: f32 = 1080.0;
// The smallest altocumulus cloudlet, as a radius in cells: about 110 m
// across (`cloud_tests.rs`).
const ALTO_SMALLEST: f32 = 0.3;

fn alto_on() -> bool {
  return max(frame.clouds5.z, frame.clouds5.w) > 0.001;
}

// The layer's wind: the cloud wind veered 20 degrees, as winds veer with
// height (`alto_wind_direction` in `gpu.rs` drifts it the same way).
fn alto_wind() -> vec2<f32> {
  let wind = frame.clouds2.zw;
  return vec2<f32>(wind.x * 0.9396926 + wind.y * 0.3420201, wind.y * 0.9396926 - wind.x * 0.3420201);
}

// The ripples of a mackerel sky: bands 900 m apart across the layer wind,
// bent by `warp` (radians). 0 in a trough, 1 on a crest.
fn alto_ripple(q: vec2<f32>, warp: f32) -> f32 {
  let wind = alto_wind();
  return 0.5 + 0.5 * sin(dot(q, vec2<f32>(-wind.y, wind.x)) * (TAU / ALTO_RIPPLE) + warp);
}

// The layer's broad pattern from one lookup, at a point on its plane: x
// the altocumulus amount there (0 to 1), y the altostratus density, z the
// ripples' warp in radians.
fn alto_coarse(xz: vec2<f32>) -> vec3<f32> {
  let q = xz + frame.alto.xy;
  let along = alto_wind();
  let across = vec2<f32>(-along.y, along.x);
  // Stretched along the wind, like the veil's fibres. At 48 km a tile, r
  // makes patches about 12 km across, g fibres about 6 km across and a
  // the ripples' bends about 3 km across.
  let local = vec2<f32>(dot(q, along) / 2.5, dot(q, across));
  let n = textureSampleLevel(noise_texture, linear_sampler, local / 48000.0 + vec2<f32>(0.29, 0.53), 0.0);
  var scale = 1.0;

  // Where the weather varies across the map, its coverage raises or
  // lowers the layer by up to 30 %, so the veil thins towards clearer
  // air.
  if (frame.regional.w > 0.5) {
    let cover = regional_weather_at(xz).r;
    scale = 1.0 + 0.3 * clamp((cover - frame.cloud_params.x) / 0.2, -1.0, 1.0);
  }

  let cumulus = saturate(frame.clouds5.z * scale * mix(0.3, 1.7, n.r));
  let stratus = frame.clouds5.w * scale * (0.7 + 0.3 * n.g);
  return vec3<f32>(cumulus, stratus, (n.a - 0.5) * 7.5);
}

// Optical depth of the layer straight through it, at a point on its
// plane, from its broad pattern alone: the cloudlets are averaged into
// their rows. For shadows, which are soft at this height anyway.
fn alto_shadow_depth(xz: vec2<f32>) -> f32 {
  let coarse = alto_coarse(xz);
  let cumulus = coarse.x * mix(0.25, 0.45, alto_ripple(xz + frame.alto.xy, coarse.z));
  return ALTO_EXTINCTION * (cumulus * ALTOCUMULUS_THICKNESS + coarse.y * ALTOSTRATUS_THICKNESS);
}

// Fraction of direct sunlight let through the mid-level layer above
// `position`.
fn alto_shadow(position: vec3<f32>) -> f32 {
  let rise = frame.alto.z - position.y;

  if (!alto_on() || rise <= 0.0) {
    return 1.0;
  }

  let sun = sun_dir();
  let sun_y = max(sun.y, 0.05);
  return exp(-alto_shadow_depth(position.xz + sun.xz * (rise / sun_y)) / sun_y);
}

// Fraction of direct sunlight that gets through the cloud layers above
// `position`. The low clouds use only the 2D weather map, which is
// exactly what shapes the volumetric clouds' footprint, so shadows line
// up with the clouds; the mid-level layer adds its own.
fn cloud_shadow(position: vec3<f32>) -> f32 {
  if (frame.cloud_colour.w < 0.5) {
    return 1.0;
  }

  var lit = 1.0;

  if (frame.cloud_params.x > 0.001) {
    let sun = sun_dir();
    let mid_height = frame.cloud_params.y + frame.cloud_params.z * 0.35;
    let travel = max(mid_height - position.y, 0.0) / max(sun.y, 0.08);
    let shadow_point = position.xz + sun.xz * travel;
    let weather = cloud_weather(shadow_point);
    let density = smoothstep(0.05, 0.6, weather) * saturate(frame.cloud_motion.w * 1.4 + 0.2);
    lit = 1.0 - density * frame.shadow_params.w;
  }

  if (alto_on()) {
    lit = lit * mix(1.0, alto_shadow(position), frame.shadow_params.w);
  }

  return lit;
}

// Sunlight blocked by hills and mountains, from the baked terrain shadow
// texture (recomputed only when the sun or terrain changes).
fn terrain_shadow(position: vec3<f32>) -> f32 {
  let strength = frame.shadow_params.z;

  if (strength <= 0.001 || world.terrain2.z < 0.5) {
    return 1.0;
  }

  let uv = (position.xz + world.terrain.xy) / max(world.terrain.xy * 2.0, vec2<f32>(1.0));

  if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
    return 1.0;
  }

  let lit = textureSampleLevel(terrain_shadow_texture, clamp_sampler, uv, 0.0).r;
  return mix(1.0, lit, strength);
}

// Tree shadows from the light-space shadow map, with a rotated four-tap
// comparison filter (each tap is itself bilinearly filtered by the
// comparison sampler, so this is effectively a 16-texel soft kernel).
fn tree_shadow(position: vec3<f32>, normal: vec3<f32>) -> f32 {
  let strength = frame.shadow_params.x;

  if (strength <= 0.001) {
    return 1.0;
  }

  // Normal offset hides self-shadowing acne without a large depth bias.
  let clip = frame.shadow_view_proj * vec4<f32>(position + normal * 0.25, 1.0);
  let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);

  if (any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0)) || clip.z >= 1.0) {
    return 1.0;
  }

  let texel = frame.shadow_params.y / f32(textureDimensions(tree_shadow_map).x);
  let depth = clip.z - 0.0008;
  var lit = 0.0;
  lit = lit + textureSampleCompareLevel(tree_shadow_map, shadow_sampler, uv + vec2<f32>(-0.7, -0.3) * texel, depth);
  lit = lit + textureSampleCompareLevel(tree_shadow_map, shadow_sampler, uv + vec2<f32>(0.3, -0.7) * texel, depth);
  lit = lit + textureSampleCompareLevel(tree_shadow_map, shadow_sampler, uv + vec2<f32>(0.7, 0.3) * texel, depth);
  lit = lit + textureSampleCompareLevel(tree_shadow_map, shadow_sampler, uv + vec2<f32>(-0.3, 0.7) * texel, depth);
  lit = lit * 0.25;
  // Fade out towards the edge of the shadowed area instead of cutting off.
  let edge = min(min(uv.x, uv.y), min(1.0 - uv.x, 1.0 - uv.y));
  lit = mix(1.0, lit, smoothstep(0.0, 0.08, edge));
  return mix(1.0, lit, strength);
}

// Combined direct-sun visibility from clouds, terrain, and trees.
fn sun_visibility(position: vec3<f32>, normal: vec3<f32>) -> f32 {
  return cloud_shadow(position) * terrain_shadow(position) * tree_shadow(position, normal);
}

// --- Fog and aerial perspective -----------------------------------------

// Integral of exp(-(h - base) / scale) along a straight segment between
// heights `h0` and `h1`, divided by the segment length.
fn height_density_average(h0: f32, h1: f32, base: f32, scale: f32) -> f32 {
  let a = exp(-clamp((h0 - base) / scale, -30.0, 60.0));
  let b = exp(-clamp((h1 - base) / scale, -30.0, 60.0));
  let dh = (h1 - h0) / scale;

  if (abs(dh) < 0.001) {
    return (a + b) * 0.5;
  }

  return (a - b) / dh;
}

struct Fog {
  inscatter: vec3<f32>,
  transmittance: f32,
};

fn mist_noise(position: vec3<f32>) -> f32 {
  let p = (position + vec3<f32>(frame.mist_wind.x, 0.0, frame.mist_wind.y)) / 420.0;
  let n = textureSampleLevel(cloud_texture, linear_sampler, p + vec3<f32>(frame.mist_wind.w), 0.0);
  return saturate(n.r * 1.25 - 0.1) * 0.75 + n.g * 0.25;
}

// Atmospheric haze plus height-based ground mist between the camera and a
// point `distance` metres along `ray`. `include_haze` is false for sky
// pixels, whose radiance already contains the atmosphere.
fn atmospheric_fog(ray: vec3<f32>, distance: f32, pixel: vec2<f32>, include_haze: bool) -> Fog {
  let camera = frame.camera_position.xyz;
  let end = camera + ray * distance;
  let sun = sun_dir();
  let mu = dot(ray, sun);
  var transmittance = vec3<f32>(1.0);
  var inscatter = vec3<f32>(0.0);

  if (include_haze) {
    // Aerial perspective: grey Mie haze whose density is set by the haze
    // distance, plus wavelength-dependent Rayleigh scattering that turns
    // distant ranges blue.
    let haze_extinction = 2.6 / max(frame.atmosphere.z, 1.0);
    let haze_depth = haze_extinction * distance * height_density_average(camera.y, end.y, 0.0, MIE_HEIGHT);
    let rayleigh_depth = rayleigh_beta() * distance * height_density_average(camera.y, end.y, 0.0, RAYLEIGH_HEIGHT);
    transmittance = exp(-(vec3<f32>(haze_depth) + rayleigh_depth));
    let horizon_ray = normalize(vec3<f32>(ray.x, max(ray.y, 0.015), ray.z));
    let haze_light = sky_radiance(horizon_ray) * 0.85 + sun_light() * henyey_greenstein(mu, 0.7) * 0.35;
    inscatter = haze_light * (vec3<f32>(1.0) - transmittance);
  }

  // Ground mist.
  let mist_density = frame.mist_params.x;

  if (mist_density > 0.0001) {
    let base = frame.mist_params.y;
    let falloff = max(frame.mist_params.z, 1.0);
    let march = min(distance, 9000.0);
    let mist_end = camera + ray * march;
    var average = height_density_average(max(camera.y, base), max(mist_end.y, base), base, falloff);

    // Mist rising off open water.
    let water_level = frame.mist_colour.w;
    average = average + height_density_average(camera.y, mist_end.y, water_level, falloff * 0.35) * 0.5
      * step(camera.y, water_level + falloff * 12.0);

    if (frame.mist_params.w > 0.001) {
      // Drifting fog banks: modulate the analytic density with a few
      // jittered samples of 3D noise along the ray.
      var noise = 0.0;
      let jitter = pixel_dither(pixel);

      for (var i = 0; i < 4; i = i + 1) {
        let t = (f32(i) + jitter) / 4.0;
        let along = pow(t, 1.6) * march;
        noise = noise + mist_noise(camera + ray * along);
      }

      average = average * mix(1.0, noise / 4.0 * 1.8, frame.mist_params.w);
    }

    // Damp shade: mist lies 20 % thicker over the ground under canopy,
    // locally, so only nearby ground reads the cover.
    if (distance < 3000.0) {
      average = average * (1.0 + 0.2 * canopy_at(end.xz));
    }
    let mist_optical = mist_density * 0.0045 * average * march;
    let mist_transmittance = exp(-mist_optical);
    let scatter = frame.mist_wind.z;
    let mist_light = frame.mist_colour.rgb
      * (sky_irradiance(vec3<f32>(0.0, 1.0, 0.0)) * 0.3 + sun_light() * (0.08 + henyey_greenstein(mu, 0.55) * scatter * 1.2));
    inscatter = inscatter * mist_transmittance + mist_light * (1.0 - mist_transmittance);
    transmittance = transmittance * mist_transmittance;
  }

  return Fog(inscatter, dot(transmittance, vec3<f32>(0.3333)));
}

// Fog applied directly by forward-shaded transparent surfaces (water).
fn apply_fog(colour: vec3<f32>, world_position: vec3<f32>, pixel: vec2<f32>) -> vec3<f32> {
  let offset = world_position - frame.camera_position.xyz;
  let distance = length(offset);
  let ray = offset / max(distance, 0.001);
  let fog = atmospheric_fog(ray, distance, pixel, true);
  return mix(colour * fog.transmittance + fog.inscatter, render_distance_colour(ray), render_distance_fog(distance));
}

// --- Render distance ----------------------------------------------------

// A smooth 0-to-1 ramp over the `length` metres before `end`; a hard step
// at `end` when `length` is 0.
fn fade_in_before(end: f32, length: f32, distance: f32) -> f32 {
  let t = saturate((distance - (end - length)) / max(length, 0.001));
  return t * t * (3.0 - 2.0 * t);
}

// Distance fog: it thickens over the fade length
// (`renderFadeMetres`) and is complete at the render distance, so the edge
// where the world stops being drawn is never seen. Beyond it, shaders skip
// their shading entirely.
fn render_distance_fog(distance: f32) -> f32 {
  let limit = frame.distances.x;

  if (limit >= 1.0e8) {
    return 0.0;
  }

  return fade_in_before(limit, frame.fades.x, distance);
}

// Whether a point is past the render distance, fully hidden by its fog.
fn beyond_render_distance(distance: f32) -> bool {
  return distance > frame.distances.x;
}

// The colour the render-distance fog fades to: the sky just above the
// horizon in the direction of `ray`.
fn render_distance_colour(ray: vec3<f32>) -> vec3<f32> {
  return sky_radiance(normalize(vec3<f32>(ray.x, 0.02, ray.z)));
}

// --- Lighting -------------------------------------------------------------

// Direct sun plus sky light on an opaque surface.
// --- Grass -----------------------------------------------------------------

// The colour of a grass blade at its base and its tip, lush to dry by
// `dryness`: `grass_instances.wgsl` shades each blade between them, and
// the terrain's grass sheen takes on their mean (`meadow_albedo`).
fn blade_base(dryness: f32) -> vec3<f32> {
  return mix(vec3<f32>(0.035, 0.075, 0.014), vec3<f32>(0.12, 0.09, 0.035), dryness);
}

fn blade_tip(dryness: f32) -> vec3<f32> {
  return mix(vec3<f32>(0.16, 0.26, 0.05), vec3<f32>(0.52, 0.4, 0.17), dryness);
}

// The mean look of a meadow's tufts, as an albedo the terrain's shading
// turns into their light, where the view ray looks `depth` deep into them
// (`meadow_depth`): their blades' colour by area (blades
// taper, so the base shows most), darkened towards the base as the tufts
// shade themselves, and the tufts' weaker sunlight (their normals lean
// only part way up). The deeper the view looks into the grass, at a slant
// or into a dense meadow, the more the tufts in front hide the lower
// parts of those behind, until the upper 40 % of the blades, lighter and
// less shaded, is most of what shows. The terrain passes the depth of a
// meadow of at most 0.9 cover: past that, blade tips crowd each other
// and hide as much as they show.
fn meadow_albedo(dryness: f32, depth: f32) -> vec3<f32> {
  let upper = 1.0 - exp(-depth / 14.0);
  return (blade_base(dryness) * mix(0.306, 0.178, upper) + blade_tip(dryness) * mix(0.261, 0.649, upper)) * MEADOW_LIGHT;
}

// The tufts' light over the ground's, for `meadow_albedo`.
const MEADOW_LIGHT: f32 = 0.78;

// Where tufts start to hand over to the ground's grass sheen, for a
// full-density radius `radius` and the grass view distance `view`:
// `grass::handover_start`.
fn handover_start(radius: f32, view: f32) -> f32 {
  return min(max(1.5 * radius, 0.3 * view), 0.9 * view);
}

// The share of a meadow's tufts drawn `distance` metres from the camera,
// with the handover starting at `start` and the grass view distance
// `view`: `grass::tuft_share`.
fn tuft_share(distance: f32, start: f32, view: f32) -> f32 {
  let handover = 1.0 - smoothstep(0.0, 1.0, (distance - start) / max(view - start, 1.0));
  let dither = 1.0 - smoothstep(0.7 * view, view, distance);
  return handover * dither;
}

// The share of the ground a meadow of `cover` hides from a view ray at
// `sine` to the ground, with its blades' sides scaled by `side`:
// `grass::apparent_cover`, with `TUFT_SIDE_AREA`.
fn apparent_cover(cover: f32, sine: f32, side: f32) -> f32 {
  return 1.0 - exp(-meadow_depth(cover, sine, side));
}

// How deep a view ray at `sine` to the ground looks into a meadow of
// `cover`: its optical depth, `-ln(1 - apparent_cover)`.
fn meadow_depth(cover: f32, sine: f32, side: f32) -> f32 {
  let s = clamp(sine, 0.02, 1.0);
  let cotangent = sqrt(1.0 - s * s) / s;
  return -log(1.0 - clamp(cover, 0.0, 0.999)) * (1.0 + 0.3862 * side * cotangent);
}

// How far the ground under a meadow takes on the tufts' look where
// `share` of them are drawn: `grass::sheen_share`.
fn sheen_share(full: f32, thinned: f32, share: f32) -> f32 {
  let drawn = 1.0 - pow(max(1.0 - thinned, 0.0001), clamp(share, 0.0, 1.0));
  return saturate((full - drawn) / max(1.0 - drawn, 0.0001));
}

fn shade_surface(
  albedo: vec3<f32>,
  normal: vec3<f32>,
  world_position: vec3<f32>,
  occlusion: f32,
  specular: f32,
  roughness: f32
) -> vec3<f32> {
  let sun = sun_dir();
  let n_dot_l = saturate(dot(normal, sun));
  let shadow = sun_visibility(world_position, normal);
  let direct = sun_light() * n_dot_l * shadow;
  let ambient = sky_irradiance(normal) * occlusion;
  var colour = albedo * (direct + ambient * 0.75) / PI * 2.6;

  if (specular > 0.001) {
    let view = normalize(frame.camera_position.xyz - world_position);
    let half_vector = normalize(view + sun);
    let n_dot_h = saturate(dot(normal, half_vector));
    let alpha = max(roughness * roughness, 0.02);
    let d = alpha * alpha / (PI * pow(n_dot_h * n_dot_h * (alpha * alpha - 1.0) + 1.0, 2.0));
    let fresnel = 0.04 + 0.96 * pow(1.0 - saturate(dot(view, half_vector)), 5.0);
    colour = colour + sun_light() * shadow * n_dot_l * d * fresnel * specular * 0.25;
  }

  return colour;
}

// --- Falling rain and snow ----------------------------------------------

// Rain streaks and snowflakes in thin depth layers around the camera, from
// 1.5 m to 48 m away, plus the grey veil a downpour draws over the
// distance. Layers are anchored to world space (arc length around the
// camera and height), so precipitation stays put when the camera turns and
// falls at real speeds, slanted by the wind. Only these few layers are
// computed per pixel, so the cost does not depend on how much of the world
// is raining. Returns light (rgb) and coverage (a).
fn precipitation(ray: vec3<f32>, max_distance: f32) -> vec4<f32> {
  let rain = frame.weather.x;
  let snow = frame.weather.y;
  let blowing = frame.cold.x;

  if (rain + snow + blowing < 0.005) {
    return vec4<f32>(0.0);
  }

  // 1 for full rain or snow; more in a storm or with `precipitationScale`
  // above 1.
  let heavy = frame.weather3.w;
  let t = time_seconds();
  let camera = frame.camera_position.xyz;
  let flat_ray = normalize(vec2<f32>(ray.x, ray.z) + vec2<f32>(0.00001, 0.0));
  // The wind across the view, taken from the camera's heading so it is the
  // same for every pixel. Taken per pixel, it would differ slightly between
  // neighbours, and multiplied by the ever-growing time it would smear
  // flakes and streaks into slivers.
  let heading = normalize(frame.camera_forward.xz + vec2<f32>(0.00001, 0.0));
  let side_wind = dot(frame.weather2.zw, vec2<f32>(-heading.y, heading.x));
  let angle = atan2(ray.z, ray.x);
  // Blowing snow skims the ground under the camera, which is close enough
  // to the ground under every layer.
  let ground = terrain_height_at(camera.xz);
  var coverage = 0.0;

  for (var layer = 0; layer < 6; layer = layer + 1) {
    let distance = 1.5 * pow(2.0, f32(layer));

    if (distance > max_distance) {
      break;
    }

    let p = camera + ray * distance;
    let arc = angle * distance;
    let fade = 1.0 - f32(layer) * 0.12;

    if (rain > 0.005) {
      // Drops fall at about 9 m/s, slanted by the wind across the view.
      let fall = p.y + t * 9.0;
      let x = arc - fall * clamp(side_wind / 9.0, -0.6, 0.6);
      let cell = vec2<f32>(floor(x / 0.09), floor(fall / 1.7));
      let local = vec2<f32>(fract(x / 0.09), fract(fall / 1.7));
      let h = hash12(cell + f32(layer) * 31.7);
      let present = step(h, saturate(rain * (0.3 + 0.3 * heavy)));
      let offset = hash12(cell + 7.3) * 0.6;
      let along = local.y - offset;
      // A streak is one drop blurred over a frame: 20 cm or so at any
      // distance, longer in a downpour. As a fraction of the 1.7 m cell:
      let span = (0.2 + 0.1 * (heavy - 1.0)) / 1.7;
      let streak = (1.0 - smoothstep(0.03, 0.09, abs(local.x - 0.5)))
        * smoothstep(0.0, span * 0.3, along) * (1.0 - smoothstep(span * 0.7, span, along));
      coverage = coverage + streak * present * (0.35 + 0.1 * heavy) * fade;
    }

    if (snow > 0.005) {
      // Flakes drift down at about a metre a second, swaying and tumbling.
      // The wind carries flakes sideways over time rather than shearing the
      // pattern as it does for rain streaks, so flakes stay round.
      let fall = p.y + t * 1.0;
      let sway = sin(t * 0.9 + fall * 0.6 + f32(layer) * 1.7) * 0.35;
      let x = arc - t * side_wind + sway;
      let cell_size = 0.22;
      let cell = vec2<f32>(floor(x / cell_size), floor(fall / cell_size));
      let local = vec2<f32>(fract(x / cell_size), fract(fall / cell_size)) * cell_size;
      let h = hash12(cell + f32(layer) * 17.3);
      let centre = (vec2<f32>(hash12(cell + 3.1), hash12(cell + 9.7)) * 0.5 + 0.25) * cell_size;
      // Flakes from 2 to 6 cm across, some clumped into larger ones.
      let radius = 0.01 + 0.02 * hash12(cell + 5.9) + 0.012 * step(0.85, hash12(cell + 2.2));
      let r = length(local - centre) / radius;
      let flake = (1.0 - smoothstep(0.55, 1.0, r)) * (0.75 + 0.25 * (1.0 - r));
      let present = step(h, saturate(snow * (0.35 + 0.2 * heavy)));
      coverage = coverage + flake * present * 0.95 * fade;
    }

    if (blowing > 0.005) {
      // Snow lifted by a gale: flat, fast streaks from the ground up to
      // about 2 m, racing along with the wind and thickest lowest down.
      let above = p.y - ground;
      let band = smoothstep(-0.4, 0.1, above) * (1.0 - smoothstep(0.6, 2.2, above));

      if (band > 0.001) {
        let x = arc - t * side_wind * 1.2;
        let y = p.y + sin(t * 1.3 + f32(layer) * 2.1) * 0.06;
        let size = vec2<f32>(0.7, 0.07);
        let cell = floor(vec2<f32>(x, y) / size);
        let local = fract(vec2<f32>(x, y) / size);
        let h = hash12(cell + f32(layer) * 13.7);
        let reach = 0.45 + 0.4 * hash12(cell + 4.1);
        let streak = (1.0 - smoothstep(0.15, 0.45, abs(local.y - 0.5)))
          * smoothstep(0.0, 0.15, local.x) * (1.0 - smoothstep(reach * 0.6, reach, local.x));
        let present = step(h, blowing * (0.3 + 0.35 * band));
        coverage = coverage + streak * present * band * 0.55 * fade;
      }
    }
  }

  // Looking steeply up or down, the layers pinch to a point and would
  // shimmer as a disc; real rain seen from below shows few streaks anyway.
  coverage = saturate(coverage) * (1.0 - smoothstep(0.72, 0.96, abs(ray.y)));

  // A downpour or heavy snow hides the distance behind a grey veil: about
  // 3 km visibility in a storm, 10 km in ordinary rain.
  let veil_density = rain * max(heavy - 0.8, 0.0) * 0.00045 + snow * heavy * 0.00025;
  let veil = 1.0 - exp(-min(max_distance, 20000.0) * veil_density);
  let light = sky_irradiance(vec3<f32>(0.0, 1.0, 0.0)) * 0.35 + sun_light() * 0.08;
  let tint = select(vec3<f32>(0.75, 0.8, 0.85), vec3<f32>(1.0), snow + blowing > rain);
  // The veil takes the colour of the air near the horizon, not of the
  // brighter sky overhead, so the distance darkens rather than glows.
  let veil_light = sky_radiance(normalize(vec3<f32>(flat_ray.x, 0.15, flat_ray.y)));
  let alpha = 1.0 - (1.0 - coverage) * (1.0 - veil);
  let colour = (light * tint * coverage + veil_light * veil * (1.0 - coverage)) / max(alpha, 0.0001);
  return vec4<f32>(colour, alpha);
}
