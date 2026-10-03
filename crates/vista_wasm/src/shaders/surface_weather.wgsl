// Wet and snowy ground that follows the weather where it fell: the GPU
// twin of `weather/surface.rs`'s `step`, for every texel of the surface
// weather map, run every quarter of a second with the weather time since
// the last run.
//
// The map holds r wetness, g puddle water (the hollow's fill times how
// much it can hold), b snow depth and a the hollow's fill. It is rgba8, so
// a quarter-second's drying is far below one step of 1/255: each value is
// rounded up or down at random in proportion, which keeps the average
// exact and leaves no visible grain.

struct Params {
  // x: seconds to advance, y: sunlight (0 to 1), z: wind (m/s), w: the
  // weather's temperature offset (°C).
  step: vec4<f32>,
  // x: precipitation everywhere when there is no regional map, y: 1 when
  // the regional map is read, z: 1 / its size in metres, w: crown area
  // per square metre per unit of cover share.
  rain: vec4<f32>,
  // x: 1 to settle every texel instead of stepping, to yzw (wetness,
  // puddle fill, snow depth) in full rain or snow, less where less falls.
  settle: vec4<f32>,
  // xy: height texels per output texel, z: metres per height texel, w: a
  // counter that changes every run, for the rounding.
  grid: vec4<f32>,
  // xy: terrain half extents in metres, z: the preset's mean
  // precipitation, for settling.
  extent: vec4<f32>,
};

@group(0) @binding(0) var previous: texture_2d<f32>;
@group(0) @binding(1) var output: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var heights: texture_2d<f32>;
// Layer 0 the surface texture (r temperature unit), layer 2 tree cover
// (r the share of the density the land holds, as sqrt(share / 4)).
@group(0) @binding(3) var ground: texture_2d_array<f32>;
@group(0) @binding(4) var regional: texture_2d<f32>;
@group(0) @binding(5) var<uniform> params: Params;

const WET_RATE: f32 = 1.0 / 120.0;
const EVAPORATION: f32 = 1.0 / 2400.0;
const PUDDLE_FILL: f32 = 1.0 / 180.0;
const PUDDLE_EVAPORATION: f32 = 0.3;
const SNOW_RATE: f32 = 1.0 / 150.0;
const MELT_RATE: f32 = 1.0 / 1800.0;
const PUDDLE_THRESHOLD: f32 = 0.6;
const PUDDLE_SLOPE_DEGREES: f32 = 3.0;

fn hash_texel(x: u32, y: u32) -> f32 {
  var h = (x * 0x8da6b343u) ^ (y * 0xd8163841u);
  h = h ^ (h >> 13u);
  h = h * 0x5bd1e995u;
  h = h ^ (h >> 15u);
  return f32(h & 0xffffu) / 65535.0;
}

fn height_at(texel: vec2<i32>) -> f32 {
  let size = vec2<i32>(textureDimensions(heights));
  return textureLoad(heights, clamp(texel, vec2<i32>(0), size - 1), 0).r;
}

fn snow_fraction(celsius: f32) -> f32 {
  return 1.0 - smoothstep(0.0, 1.0, (celsius - 0.5) / 2.0);
}

fn unit_smoothstep(t: f32) -> f32 {
  let x = clamp(t, 0.0, 1.0);
  return x * x * (3.0 - 2.0 * x);
}

fn puddle_capacity(slope_degrees: f32, hollow: f32) -> f32 {
  let level = 1.0 - unit_smoothstep((slope_degrees - PUDDLE_SLOPE_DEGREES * 0.6) / (PUDDLE_SLOPE_DEGREES * 0.4));
  return level * unit_smoothstep(hollow / 0.05);
}

// The regional map's precipitation at a world position, bilinearly.
fn regional_rain(xz: vec2<f32>) -> f32 {
  let size = vec2<f32>(textureDimensions(regional));
  let texel = clamp((xz * params.rain.z + 0.5) * size - 0.5, vec2<f32>(0.0), size - 1.0);
  let base = vec2<i32>(floor(texel));
  let f = texel - floor(texel);
  let top = mix(textureLoad(regional, base, 0).g, textureLoad(regional, min(base + vec2<i32>(1, 0), vec2<i32>(size) - 1), 0).g, f.x);
  let bottom = mix(textureLoad(regional, min(base + vec2<i32>(0, 1), vec2<i32>(size) - 1), 0).g, textureLoad(regional, min(base + vec2<i32>(1, 1), vec2<i32>(size) - 1), 0).g, f.x);
  return mix(top, bottom, f.y);
}

// Round to the nearest of the two rgba8 steps either side, at random in
// proportion to the distance.
fn dither(value: vec4<f32>, random: vec4<f32>) -> vec4<f32> {
  let scaled = clamp(value, vec4<f32>(0.0), vec4<f32>(1.0)) * 255.0;
  return (floor(scaled) + step(random, fract(scaled))) / 255.0;
}

@compute @workgroup_size(8, 8, 1)
fn step_main(@builtin(global_invocation_id) id: vec3<u32>) {
  let size = textureDimensions(output);

  if (id.x >= size.x || id.y >= size.y) {
    return;
  }

  let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(size);
  let xz = (uv * 2.0 - 1.0) * params.extent.xy;
  let height_texel = vec2<i32>(floor(uv * vec2<f32>(textureDimensions(heights))));
  let centre = height_at(height_texel);
  let west = height_at(height_texel - vec2<i32>(1, 0));
  let east = height_at(height_texel + vec2<i32>(1, 0));
  let north = height_at(height_texel - vec2<i32>(0, 1));
  let south = height_at(height_texel + vec2<i32>(0, 1));
  let metres = max(params.grid.z, 0.001);
  let gradient = length(vec2<f32>(east - west, south - north)) / (2.0 * metres);
  let slope_degrees = degrees(atan(gradient));
  let hollow = (west + east + north + south) * 0.25 - centre;
  let capacity = puddle_capacity(slope_degrees, hollow);

  let ground_texel = vec2<i32>(floor(uv * vec2<f32>(textureDimensions(ground))));
  let celsius = textureLoad(ground, ground_texel, 0, 0).r * 65.0 - 30.0 + params.step.w;
  let red = textureLoad(ground, ground_texel, 2, 0).r * 255.0;
  let canopy = 1.0 - exp(-params.rain.w * red * red * (4.0 / 65025.0));

  var precipitation = params.rain.x;

  if (params.rain.y > 0.5) {
    precipitation = regional_rain(xz);
  }

  // Settled ground is as wet as the rain that falls on it.
  if (params.settle.x > 0.5) {
    let share = saturate(precipitation / max(params.extent.z, 0.01)) * step(0.001, params.extent.z);
    let settled = params.settle.yzw * share;
    textureStore(output, vec2<i32>(id.xy), vec4<f32>(settled.x, settled.y * capacity, settled.z, settled.y));
    return;
  }

  let dt = params.step.x;
  let sun = clamp(params.step.y, 0.0, 1.0);
  let state = textureLoad(previous, vec2<i32>(id.xy), 0);
  let frozen = snow_fraction(celsius);
  let rain = max(precipitation, 0.0) * (1.0 - frozen) * (1.0 - canopy * 0.3);
  let snowfall = max(precipitation, 0.0) * frozen;
  let patchiness = hash_texel(id.x, id.y);
  let warmth = clamp(0.3 + celsius / 20.0, 0.1, 2.0);
  let dry = EVAPORATION * (0.3 + sun) * (0.5 + max(params.step.z, 0.0) / 10.0) * warmth
    * (1.0 - canopy * 0.5) * (0.6 + 0.8 * patchiness) * (1.0 + slope_degrees / 15.0);

  var snow = state.b;

  if (celsius < 0.5) {
    snow = snow + snowfall * SNOW_RATE * dt;
  }

  var melt = 0.0;

  if (celsius > 1.0) {
    melt = min(MELT_RATE * (celsius - 1.0) * (0.5 + sun) * dt, snow);
    snow = snow - melt;
  }

  let water = min(rain, 1.0) + melt / max(dt, 0.001) * 60.0;
  let wetness = state.r + (water * WET_RATE - dry) * dt;
  var filling = 0.0;

  if (max(state.r, wetness) > PUDDLE_THRESHOLD) {
    filling = water * PUDDLE_FILL;
  }

  let fill = clamp(state.a + (filling - dry * PUDDLE_EVAPORATION) * dt, 0.0, 1.0);
  let wet = max(clamp(wetness, 0.0, 1.0), fill * capacity * 0.8);
  let counter = params.grid.w;
  let random = vec4<f32>(
    hash_texel(id.x + 7919u * u32(counter), id.y),
    hash_texel(id.x, id.y + 104729u * u32(counter)),
    hash_texel(id.x ^ u32(counter), id.y + 31u),
    hash_texel(id.x + 17u, id.y ^ u32(counter))
  );
  textureStore(output, vec2<i32>(id.xy), dither(vec4<f32>(wet, fill * capacity, snow, fill), random));
}
