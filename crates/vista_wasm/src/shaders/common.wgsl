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
  // xy: snapped ocean grid origin, zw: unused.
  water_origin: vec4<f32>,
  // x: wind, y: variation, z: tree style, w: grass view distance.
  vegetation: vec4<f32>,
  // x: tree mesh distance, yzw: unused.
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
};

struct WorldInfo {
  // Per species: x height, y radius (metres), zw unused.
  species: array<vec4<f32>, 8>,
  // Per species: rgb foliage tint, w unused.
  species_tint: array<vec4<f32>, 8>,
  // xy: terrain half extents (metres), zw: metres per height texel.
  terrain: vec4<f32>,
  // xy: height texture size, z: 1 when a terrain is loaded, w: unused.
  terrain2: vec4<f32>,
  // Per material colour multiplier (rgb).
  material_tints: array<vec4<f32>, 8>,
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

fn hash12(p: vec2<f32>) -> f32 {
  var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
  p3 = p3 + dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
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
  return MIE_BETA * (0.1 + max(frame.atmosphere.y, 0.0) * 0.45);
}

fn sun_transmittance(cos_zenith: f32) -> vec3<f32> {
  let m = air_mass(cos_zenith);
  let transmittance = exp(-(rayleigh_beta() * RAYLEIGH_HEIGHT + mie_beta() * MIE_HEIGHT) * m);
  // Fade out as the sun sinks below the horizon.
  return transmittance * smoothstep(-0.08, 0.02, cos_zenith);
}

// Sunlight reaching the ground, in the same units as `sky_radiance`. A
// full overcast diffuses most direct light into the sky's ambient.
fn sun_light() -> vec3<f32> {
  let overcast = 1.0 - frame.weather2.y * 0.9;
  return sun_transmittance(sun_dir().y) * frame.sun_direction.w * 2.35 * overcast;
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
  let phase_m = henyey_greenstein(mu, 0.76);
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
  // Weather: overcast skies turn a flat, darker grey; lightning lights the
  // whole sky for a moment.
  let overcast = frame.weather2.y;
  sky = mix(sky, vec3<f32>(luminance(sky)) * vec3<f32>(0.92, 0.95, 1.0) * (1.0 - overcast * 0.45), overcast);
  return sky + vec3<f32>(0.75, 0.8, 1.0) * frame.weather2.x * 1.6;
}

// Diffuse sky light arriving on a surface with normal `normal`.
fn sky_irradiance(normal: vec3<f32>) -> vec3<f32> {
  let up = sky_radiance(vec3<f32>(0.0, 1.0, 0.0));
  let horizon = sky_radiance(normalize(vec3<f32>(-sun_dir().x, 0.2, -sun_dir().z)));
  let sky = mix(horizon, up, saturate(normal.y * 0.5 + 0.5));
  // Light bounced off the ground below.
  let bounce = sun_light() * 0.035 * saturate(-normal.y * 0.5 + 0.5);
  return sky * (0.55 + 0.45 * saturate(normal.y * 0.5 + 0.5)) * 2.4 + bounce;
}

// --- Terrain height lookups ---------------------------------------------

fn terrain_height_at(xz: vec2<f32>) -> f32 {
  if (world.terrain2.z < 0.5) {
    return -100000.0;
  }

  let size = world.terrain2.xy;
  let texel = (xz + world.terrain.xy) / world.terrain.zw;
  let clamped = clamp(texel, vec2<f32>(0.0), size - vec2<f32>(1.001));
  let base = floor(clamped);
  let f = clamped - base;
  let i = vec2<i32>(base);
  let h00 = textureLoad(height_texture, i, 0).r;
  let h10 = textureLoad(height_texture, i + vec2<i32>(1, 0), 0).r;
  let h01 = textureLoad(height_texture, i + vec2<i32>(0, 1), 0).r;
  let h11 = textureLoad(height_texture, i + vec2<i32>(1, 1), 0).r;
  var h = mix(mix(h00, h10, f.x), mix(h01, h11, f.x), f.y);
  // Outside the terrain footprint the sea floor keeps sloping down, so the
  // open ocean beyond the map edge reads as deep water.
  let outside = max(abs(xz) - world.terrain.xy, vec2<f32>(0.0));
  h = h - length(outside) * 0.08;
  return h;
}

// --- Clouds (shared by the sky pass and cloud shadows) -------------------

fn cloud_weather(xz: vec2<f32>) -> f32 {
  let coverage = frame.cloud_params.x;
  let uv = (xz + frame.cloud_motion.xy) / 26000.0;
  let large = textureSampleLevel(noise_texture, linear_sampler, uv, 0.0).r;
  let medium = textureSampleLevel(noise_texture, linear_sampler, uv * 3.3 + vec2<f32>(0.37, 0.71), 0.0).g;
  let weather = large * 0.7 + medium * 0.3;
  return saturate(remap(weather, 1.0 - coverage * 0.95 - 0.05, 1.0 - coverage * 0.35, 0.0, 1.0));
}

// Fraction of direct sunlight that gets through the cloud layer above
// `position`. Uses only the 2D weather map, which is exactly what shapes
// the volumetric clouds' footprint, so shadows line up with the clouds.
fn cloud_shadow(position: vec3<f32>) -> f32 {
  if (frame.cloud_params.x <= 0.001 || frame.cloud_colour.w < 0.5) {
    return 1.0;
  }

  let sun = sun_dir();
  let mid_height = frame.cloud_params.y + frame.cloud_params.z * 0.35;
  let travel = max(mid_height - position.y, 0.0) / max(sun.y, 0.08);
  let shadow_point = position.xz + sun.xz * travel;
  let weather = cloud_weather(shadow_point);
  let density = smoothstep(0.05, 0.6, weather) * saturate(frame.cloud_motion.w * 1.4 + 0.2);
  return 1.0 - density * frame.shadow_params.w;
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
  let fog = atmospheric_fog(offset / max(distance, 0.001), distance, pixel, true);
  return colour * fog.transmittance + fog.inscatter;
}

// --- Lighting -------------------------------------------------------------

// Direct sun plus sky light on an opaque surface.
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
