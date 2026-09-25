// Renders the ocean, rivers, lakes and waterfalls. `common.wgsl` is
// prepended.
//
// Ocean: a camera-following grid displaced by a sum of eight Gerstner
// waves spread around the configured swell direction. Waves shoal (shrink,
// then break into foam) as the water gets shallow, and each wave fades out
// where the grid is too coarse to represent it, so distant water never
// aliases. Rivers: ribbons whose vertices carry the current (Manning speed
// along the channel), the valley slope, the bend and the depth; the detail
// normal map is advected along the current with a two-phase flow map, glassy
// when slow and choppy when fast, with standing waves and whitewater on
// rapids, foam on the outer bank of bends, and silt clouding fast rivers.
// Ribbons are wider than the water and fade out by depth, so banks meet the
// water with no edge. Lakes: flat surfaces rippled by the wind-driven
// current. Snowmelt (`frame.rivers.x`) makes rivers faster, foamier and a
// little higher in their channels.
//
// Waterfalls (behind the uniform guard `falls_possible()`): a curved sheet
// with streaks falling at the impact speed, aerated white towards its
// foot, thin and translucent at its edges and lit through from behind;
// camera-facing mist sprites rising from the foot; and a plunge pool of
// churned foam rings flowing outwards.
//
// Cold lakes, rivers and falls freeze (behind `freezing_possible()`):
// lakes below 0 °C as snow-covered lake ice with cracks and clear dark
// patches, rivers below -5 °C as snow-dusted ice with open leads over the
// fastest water, and falls below -8 °C as ribbed icefalls with no spray.
// Freezing is partial over the 2 °C above that, margins first.
//
// Cold seas freeze. Ice concentration comes from the surface texture's
// temperature over the terrain and from the open-sea temperature beyond
// it: open water above -1.5 °C, full pack ice below -7.5 °C, and fast ice
// frozen to the shore where the climate is colder than -10 °C. The pack
// (see `pack_ice`) has floes of many sizes, leads, pressure ridges and
// brash, drifting with the wind; waves and foam die down between them.
//
// Shading uses real water depth from the terrain height texture for
// absorption (turquoise shallows, dark deep water, visible sea bed), a
// Schlick Fresnel reflection of the same analytic sky as the sky pass, a
// GGX sun glitter, subsurface light through wave crests, and foam on
// crests, shorelines, and rapids.

struct VertexIn {
  @location(0) position: vec3<f32>,
  @location(1) flow: vec2<f32>,
  @location(2) params: vec3<f32>,
  // See `WaterVertex::extra` in `render/water.rs`.
  @location(3) extra: vec4<f32>,
};

struct VertexOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) world_position: vec3<f32>,
  @location(1) flow: vec2<f32>,
  @location(2) @interpolate(flat) kind: i32,
  @location(3) across: f32,
  @location(4) spacing: f32,
  @location(5) rest_xz: vec2<f32>,
  @location(6) extra: vec4<f32>,
  // True width over drawn width, for ribbons widened to stay visible.
  @location(7) coverage: f32,
};

const KIND_OCEAN: i32 = 0;
const KIND_RIVER: i32 = 1;
const KIND_LAKE: i32 = 2;
const KIND_FALL: i32 = 3;
const KIND_SPRAY: i32 = 4;
const KIND_POOL: i32 = 5;

// True in the pipelines that draw rivers, lakes, pools and waterfalls,
// false in those that draw the ocean, so each compiles without the
// other's code.
override INLAND: bool = false;

// The opaque scene at half resolution: HDR colour, and linear view depth
// in alpha (see `scene_copy_main` in `atmosphere.wgsl`).
@group(3) @binding(0) var scene_copy: texture_2d<f32>;

// Whether any waterfall exists this frame.
fn falls_possible() -> bool {
  return frame.rivers.z > 0.5;
}

// Whether any lake, river or waterfall may be frozen this frame.
fn freezing_possible() -> bool {
  return frame.rivers.y > 0.5;
}

// How much is frozen, where freezing starts at `start` °C and is complete
// 2 °C colder. The CPU twin is `freeze_fraction` in `render/water.rs`.
fn freeze_fraction(celsius: f32, start: f32) -> f32 {
  return saturate((start - celsius) / 2.0);
}

const WAVE_COUNT: i32 = 8;

struct Wave {
  direction: vec2<f32>,
  k: f32,
  amplitude: f32,
  omega: f32,
  steepness: f32,
  phase: f32,
  wavelength: f32,
};

fn wave(i: i32) -> Wave {
  var scales = array<f32, 8>(1.0, 0.61, 0.43, 0.29, 0.21, 0.14, 0.095, 0.061);
  var angles = array<f32, 8>(0.0, 0.55, -0.42, 1.05, -0.95, 0.28, -1.45, 1.6);
  var phases = array<f32, 8>(0.0, 1.7, 4.1, 2.3, 5.2, 0.9, 3.3, 5.9);
  let base_wavelength = max(frame.wave_params.y, 0.5);
  let angle = frame.wave_params.z + angles[i] * frame.wave_params2.y;
  let wavelength = base_wavelength * scales[i];
  let k = TAU / wavelength;
  // Secondary waves keep roughly the same steepness as the swell.
  let amplitude = frame.wave_params.x * 0.5 * pow(scales[i], 1.1) * select(0.55, 1.0, i == 0);
  let omega = sqrt(9.81 * k) * frame.wave_params2.x;
  let steepness = saturate(frame.wave_params.w) / max(k * amplitude * f32(WAVE_COUNT), 0.0001);
  return Wave(
    vec2<f32>(sin(angle), cos(angle)),
    k,
    amplitude,
    omega,
    min(steepness, 1.0),
    phases[i],
    wavelength
  );
}

// How much of a wave survives at this depth and grid resolution.
fn wave_attenuation(w: Wave, depth: f32, spacing: f32) -> f32 {
  let shoal = smoothstep(0.0, max(w.wavelength * 0.06, 0.6), depth);
  let resolvable = 1.0 - smoothstep(w.wavelength / 7.0, w.wavelength / 3.5, spacing);
  return shoal * resolvable;
}

struct WaveSample {
  displacement: vec3<f32>,
  normal: vec3<f32>,
  jacobian: f32,
  height: f32,
};

fn sample_waves(xz: vec2<f32>, depth: f32, spacing: f32) -> WaveSample {
  var result = WaveSample(vec3<f32>(0.0), vec3<f32>(0.0, 1.0, 0.0), 1.0, 0.0);

  if (frame.wave_params2.z < 0.5 || frame.wave_params.x <= 0.0001) {
    return result;
  }

  let t = time_seconds();
  var nx = 0.0;
  var nz = 0.0;
  var ny = 1.0;

  for (var i = 0; i < WAVE_COUNT; i = i + 1) {
    let w = wave(i);
    let a = w.amplitude * wave_attenuation(w, depth, spacing);

    if (a <= 0.0001) {
      continue;
    }

    let phase = w.k * dot(w.direction, xz) - w.omega * t + w.phase;
    let c = cos(phase);
    let s = sin(phase);
    let qa = w.steepness * a;
    result.displacement = result.displacement + vec3<f32>(qa * w.direction.x * c, a * s, qa * w.direction.y * c);
    nx = nx - w.direction.x * w.k * a * c;
    nz = nz - w.direction.y * w.k * a * c;
    ny = ny - w.steepness * w.k * a * s;
    result.height = result.height + s * a;
  }

  result.normal = normalize(vec3<f32>(nx, max(ny, 0.05), nz));
  result.jacobian = ny;
  return result;
}

// False in the pipeline drawn when no sea can freeze, so that pipeline
// compiles without any of the sea ice code.
override SEA_ICE: bool = true;

// Whether sea ice may form this frame.
fn sea_ice_possible() -> bool {
  return SEA_ICE && frame.sea_ice.x > 0.5;
}

// Sea ice concentration on the ocean, 0 (open water) to 1 (full pack).
fn sea_ice_concentration(xz: vec2<f32>) -> f32 {
  if (!sea_ice_possible()) {
    return 0.0;
  }

  var unit = frame.sea_ice.y;
  var fast = 0.0;

  if (over_terrain(xz)) {
    // Fade into the open-sea temperature over the last 300 m of terrain,
    // so there is no seam at its edge.
    let margin = world.terrain.xy - abs(xz);
    let inside = saturate(min(margin.x, margin.y) / 300.0);
    let surface = surface_at(xz);
    unit = mix(unit, surface.r, inside);
    fast = surface.b * inside;
  }

  let celsius = unit * 65.0 - 30.0;
  return max(saturate((-1.5 - celsius) / 6.0), fast);
}

struct FloeCell {
  // Distance to the nearest and second-nearest cell centres, in cells.
  f1: f32,
  f2: f32,
  // Random values of the nearest and second-nearest cells.
  id: f32,
  pair: f32,
  // The warp noise, reused for drifted snow on the smallest scale.
  grain: f32,
  // Unit direction from the nearest cell centre to the point.
  outward: vec2<f32>,
};

// Three random values for a cell, from one hash.
fn floe_hash(cell: vec2<f32>) -> vec3<f32> {
  var p3 = fract(vec3<f32>(cell.x, cell.y, cell.x) * vec3<f32>(0.1031, 0.103, 0.0973));
  p3 = p3 + dot(p3, p3.yxz + 33.33);
  return fract((p3.xxy + p3.yzz) * p3.zyx);
}

fn floe_cell(p: vec2<f32>) -> FloeCell {
  let base = floor(p);
  var result = FloeCell(8.0, 8.0, 0.0, 0.0, 0.5, vec2<f32>(0.0, 1.0));

  for (var y = -1; y <= 1; y = y + 1) {
    for (var x = -1; x <= 1; x = x + 1) {
      let cell = base + vec2<f32>(f32(x), f32(y));
      let random = floe_hash(cell);
      let offset = p - cell - random.xy * 0.8 - 0.1;
      let d = length(offset);
      let id = random.z;

      if (d < result.f1) {
        result.f2 = result.f1;
        result.pair = result.id;
        result.f1 = d;
        result.id = id;
        result.outward = offset / max(d, 0.0001);
      } else if (d < result.f2) {
        result.f2 = d;
        result.pair = id;
      }
    }
  }

  return result;
}

// One scale of floes: Worley cells whose lookup is warped by noise with
// wavelengths of one and half a cell, by up to 0.3 of a cell, so floe
// edges are rounded and irregular, never straight polygon sides. The
// noise is sampled at the mip level of the pixel's footprint, so distant
// floes do not shimmer.
fn floe_layer(p: vec2<f32>, size: f32, footprint: f32) -> FloeCell {
  let span = size * 4.0;
  let lod = max(log2(footprint * 512.0 / span), 0.0);
  let warp = textureSampleLevel(noise_texture, linear_sampler, p / span, lod).rg - 0.5;
  var cell = floe_cell(p / size + warp * 0.6);
  cell.grain = warp.x + 0.5;
  return cell;
}

struct Floes {
  // How much of the pixel is ice, 0 to 1, and how much of the water
  // between floes is grey brash and grease ice.
  cover: f32,
  brash: f32,
  // Bevelled rim, 0 to 1, and the direction it slopes down.
  rim: f32,
  outward: vec2<f32>,
  // Pressure ridge, 0 to 1, and the direction it rises from.
  ridge: f32,
  ridge_side: vec2<f32>,
  // Per-floe colour variation, and how much bare ice shows through thin,
  // patchy snow.
  shade: f32,
  bare: f32,
  // Drifted snow, 0 to 1, lighter and darker patches a few metres across.
  grain: f32,
};

// Pack ice at a concentration: big floes 600 m across, broken by a 120 m
// scale into bays and loose pieces, with 25 m cakes and brash in the gaps,
// so floe sizes span orders of magnitude; long leads of open water that
// follow a slowly turning direction; and pressure ridges along some floe
// boundaries. Beyond a few kilometres the small scales fade out, and the
// big floes, their colours and the leads carry the look.
// `p` already includes any drift. Lake ice calls it on a scaled position,
// which makes every floe, crack and lead proportionally larger.
fn pack_ice(p: vec2<f32>, concentration: f32, footprint: f32, distance: f32) -> Floes {
  let c = concentration;
  var floes = Floes(0.0, 0.0, 0.0, vec2<f32>(0.0, 1.0), 0.0, vec2<f32>(0.0, 1.0), 0.0, 0.0, 0.5);

  let large = floe_layer(p, 600.0, footprint);
  let large_edge = large.f2 - large.f1;
  let large_blur = footprint / 600.0;
  // Big floes stay apart even in a closed pack, by a gap of slush that
  // widens as the pack opens, so their outlines read from far away.
  let gap = 0.03 + (1.0 - c) * 0.03;
  floes.cover = step(large.id, c) * smoothstep(gap, gap + 0.02 + large_blur, large_edge);
  floes.rim = 1.0 - smoothstep(0.0, 0.05 + large_blur, large_edge);
  floes.outward = large.outward;
  floes.shade = fract(large.id * 7.7);
  // One floe in five carries only thin, patchy snow.
  let thin = step(fract(large.id * 13.7), 0.2);

  let middle_weight = 1.0 - smoothstep(4000.0, 8000.0, distance);

  if (middle_weight > 0.01 && floes.cover > 0.0) {
    let middle = floe_layer(p + 37.0, 120.0, footprint);
    let middle_edge = middle.f2 - middle.f1;
    let blur = footprint / 120.0;
    // Some middle cells have broken away, leaving bays and loose pieces;
    // others are split off from their floe by cracks.
    let broken = step(middle.id, 0.08 + 0.3 * (1.0 - c));
    let cracked = step(middle.id, 0.35);
    let split = mix(1.0, smoothstep(0.0, 0.02 + blur, middle_edge), cracked);
    floes.cover = floes.cover * mix(1.0, (1.0 - broken) * split, middle_weight);
    // Pressure ridges where floes pushed together: along a quarter of the
    // boundaries, chosen by both cells so the ridge rises on both sides.
    floes.ridge = step(0.75, fract((middle.id + middle.pair) * 7.3)) * (1.0 - smoothstep(0.0, 0.06 + blur, middle_edge))
      * floes.cover * middle_weight;
    floes.ridge_side = middle.outward;
    floes.bare = thin * step(0.45, fract(middle.id * 5.31)) * middle_weight;
  }

  let small_weight = 1.0 - smoothstep(1500.0, 3000.0, distance);

  // Cakes only matter in the gaps; inside a floe, only its snow grain.
  if (small_weight > 0.01 && floes.cover < 0.99) {
    let small = floe_layer(p + 91.0, 25.0, footprint);
    let cake = step(small.id, c * 0.75)
      * smoothstep(0.0, 0.02 + footprint / 25.0, small.f2 - small.f1);
    floes.cover = floes.cover + (1.0 - floes.cover) * cake * small_weight;
    floes.grain = mix(0.5, small.grain, small_weight);
  } else if (small_weight > 0.01) {
    let grain = textureSampleLevel(noise_texture, linear_sampler, (p + 91.0) / 100.0, max(log2(footprint * 5.12), 0.0)).r;
    floes.grain = mix(0.5, grain, small_weight);
  }

  // Leads: the contour lines of noise stretched along one direction,
  // meandering with a wavelength of about 2 km, broken into long segments,
  // fewer and narrower as the pack closes up.
  let across = dot(p, vec2<f32>(-0.6, 0.8));
  let along = dot(p, vec2<f32>(0.8, 0.6));
  let bend = 150.0 * sin(along * 0.003 + 0.8 * sin(across * 0.0009)) + 90.0 * sin(along * 0.0011 + 1.7);
  let q = vec2<f32>(along / 16000.0, (across + bend) / 2000.0);
  let lead_noise = textureSampleLevel(noise_texture, linear_sampler, q, max(log2(footprint * 0.256), 0.0));
  let width = mix(0.032, 0.011, c) + footprint * 0.00014;
  let lead = (1.0 - smoothstep(width * 0.5, width, abs(lead_noise.r - 0.5)))
    * smoothstep(0.2, 0.3, lead_noise.g - c * 0.35);
  floes.cover = floes.cover * (1.0 - lead);
  floes.ridge = floes.ridge * (1.0 - lead);
  // Brash and grease ice fill the rest in a close pack, not black water:
  // the share of the water between floes that is slush.
  floes.brash = (1.0 - lead) * smoothstep(0.55, 0.95, c);
  return floes;
}

@vertex
fn vertex_main(in: VertexIn) -> VertexOut {
  let kind = i32(in.params.x + 0.5);
  var position = in.position;

  let ocean = !INLAND && kind == KIND_OCEAN;

  if (ocean) {
    position = vec3<f32>(in.position.x + frame.water_origin.x, frame.water_shallow.w, in.position.z + frame.water_origin.y);
  }

  var out: VertexOut;
  out.rest_xz = position.xz;
  out.flow = in.flow;
  out.extra = in.extra;

  if (ocean) {
    let depth = position.y - terrain_height_at(position.xz);
    let waves = sample_waves(position.xz, depth, in.params.z);
    var displacement = waves.displacement;

    // Pack ice damps the swell.
    if (sea_ice_possible()) {
      displacement = displacement * (1.0 - sea_ice_concentration(position.xz));
    }

    position = position + displacement;
  }

  out.coverage = 1.0;

  // Snowmelt fills rivers within their channels: up to a fifth of their
  // depth higher at full melt. A far ribbon narrower than a pixel breaks
  // into dashes, so each side is widened to at least 0.75 pixel, and its
  // alpha scaled by how much of that the water covers.
  if (INLAND && kind == KIND_RIVER) {
    position.y = position.y + 0.2 * in.extra.z * saturate((frame.rivers.x - 1.0) / 0.4);
    let footprint = distance(position, frame.camera_position.xyz) * frame.camera_forward.w * 2.0 * frame.viewport.w;
    let drawn = max(in.params.z, 0.75 * footprint);
    let side = normalize(vec2<f32>(-in.flow.y, in.flow.x));
    position = position + vec3<f32>(side.x, 0.0, side.y) * (drawn - in.params.z) * in.params.y;
    out.coverage = in.params.z / drawn;
  }

  // Mist sprites rise from the foot of a fall and drift downwind, then
  // start again. The motion comes only from the frame time and constants
  // of the sprite, so every pixel of it agrees.
  if (INLAND && kind == KIND_SPRAY && falls_possible()) {
    let seed = in.params.y;
    let size = in.params.z;
    let height = in.extra.x;
    let life = 3.0 + seed * 3.0;
    let age = fract(time_seconds() / life + seed * 7.13);
    let rise = age * min(height * 0.6, size * 2.0) + size * 0.3;
    let centre = in.position + vec3<f32>(frame.weather2.z, 0.0, frame.weather2.w) * age * life * 0.35 + vec3<f32>(0.0, rise, 0.0);
    let grown = size * (0.6 + age * 0.8);
    let distance = length(centre - frame.camera_position.xyz);
    // Beyond 1.5 km, or when the fall has frozen, the sprite collapses to
    // a point and draws nothing.
    let hidden = distance > 1500.0 || (freezing_possible() && freeze_fraction(in.extra.z, -8.0) > 0.5);
    let corner = select(in.flow, vec2<f32>(0.0), hidden);
    position = centre + (frame.camera_right.xyz * corner.x + frame.camera_up.xyz * corner.y) * grown * 0.5;
    out.extra = vec4<f32>(age, grown, sqrt(max(in.extra.x * in.extra.y, 0.0)), distance);
  }

  out.clip_position = frame.view_proj * vec4<f32>(position, 1.0);
  out.world_position = position;
  out.kind = kind;
  out.across = in.params.y;
  out.spacing = in.params.z;
  return out;
}

// Only called in uniform control flow.
fn detail_normal(uv: vec2<f32>) -> vec3<f32> {
  let texel = textureSample(water_texture, linear_sampler, uv);
  return vec3<f32>(texel.r * 2.0 - 1.0, 0.0, texel.g * 2.0 - 1.0);
}

// Mip level for a 512-texel texture repeating every `tile` metres, seen
// with `footprint` metres per pixel. Safe in any control flow.
fn texture_lod(footprint: f32, tile: f32) -> f32 {
  return max(log2(footprint * 512.0 / tile), 0.0);
}

// Lake and river ice: colour (rgb) and how much of the pixel it covers
// (a). Lakes freeze below 0 °C and rivers below -5 °C, fully 2 °C colder,
// shallow margins first. Lake ice is the pack ice drawn at four times
// the scale: great smooth sheets meeting at pressure cracks. Snow lies
// deeper the colder it is, blown thin in patches where clear, dark ice
// shows. Rivers keep open leads over their fastest water.
fn frozen_surface(
  in: VertexOut,
  position: vec3<f32>,
  depth: f32,
  speed: f32,
  footprint: f32,
  distance: f32
) -> vec4<f32> {
  let river = in.kind == KIND_RIVER;
  let celsius = in.extra.w;
  // Lakes start to freeze at 0 °C, rivers at -5 °C and the churned pools
  // below waterfalls, with their falls, at -8 °C.
  let start = select(select(0.0, -8.0, in.kind == KIND_POOL), -5.0, river);
  let concentration = freeze_fraction(celsius, start);

  if (concentration <= 0.001) {
    return vec4<f32>(0.0);
  }

  let margin = 1.0 - saturate(depth / max(select(3.0, in.extra.z, river), 0.3));
  var cover = saturate((concentration - (1.0 - margin) * 0.8) * 6.0);
  var crack = 0.0;

  if (river) {
    let lead = (1.0 - smoothstep(0.25, 0.6, abs(in.across))) * smoothstep(2.0, 2.5, speed);
    cover = cover * (1.0 - lead);
  } else {
    let sheets = pack_ice(in.rest_xz / 4.0, 1.0, footprint / 4.0, distance);
    crack = (1.0 - sheets.cover) * (1.0 - sheets.brash * 0.5) + sheets.ridge * 0.5;
  }

  let drift = textureSampleLevel(noise_texture, linear_sampler, in.rest_xz / 70.0, texture_lod(footprint, 70.0));
  // Wind scours snow off the ice in patches.
  let snow = saturate(saturate((-1.0 - celsius) / 10.0) * 1.4 - (drift.r - 0.5) * 1.2 - drift.g * 0.3);
  let clear_ice = vec3<f32>(0.02, 0.035, 0.05);
  var albedo = mix(clear_ice, vec3<f32>(0.84, 0.87, 0.92) * (0.9 + drift.b * 0.1), snow);
  albedo = mix(albedo, vec3<f32>(0.03, 0.05, 0.07), saturate(crack) * (1.0 - snow * 0.6));
  let colour = shade_surface(albedo, vec3<f32>(0.0, 1.0, 0.0), position, 1.0, (1.0 - snow) * 0.9, mix(0.06, 0.6, snow));
  return vec4<f32>(colour, cover);
}

// Waterfall sheets and their mist, drawn by a pipeline of their own so
// the rest of the water never pays for them.
@fragment
fn fragment_fall(in: VertexOut) -> @location(0) vec4<f32> {
  let position = in.world_position;
  let view_offset = frame.camera_position.xyz - position;
  let distance = length(view_offset);
  let view = view_offset / max(distance, 0.001);
  let sun = sun_dir();
  let t = time_seconds();
  let footprint = distance * frame.camera_forward.w * 2.0 * frame.viewport.w;

  if (in.kind == KIND_SPRAY) {
    let r2 = dot(in.flow, in.flow);

    // Outside the disc: fully transparent. No `discard`, which would cost
    // the whole water pass its early depth test.
    if (r2 >= 1.0) {
      return vec4<f32>(0.0);
    }

    let age = in.extra.x;
    let size = in.extra.y;
    // Soft where the sprite meets the ground, instead of a hard line.
    let ground = terrain_height_at(position.xz);
    let soft = saturate((position.y - ground) / max(size * 0.25, 0.5)) * saturate((distance - 1.0) / 4.0);
    let far = 1.0 - smoothstep(1200.0, 1500.0, in.extra.w);
    let billow = textureSampleLevel(noise_texture, linear_sampler, (position.xz + position.y) / max(size * 2.0, 1.0) + in.flow * 0.2, 0.0).r;
    let puff = (1.0 - r2) * (1.0 - r2) * mix(0.55, 1.0, billow);
    let density = puff * (1.0 - age) * smoothstep(0.0, 0.15, age) * saturate(in.extra.z * 0.1) * soft * far;
    let forward = henyey_greenstein(dot(-view, sun), 0.6);
    let colour = vec3<f32>(0.92) * (sun_light() * (0.25 + forward * 1.8) + sky_irradiance(vec3<f32>(0.0, 1.0, 0.0)) * 0.4) / PI * 2.2;
    return vec4<f32>(finish_colour(apply_fog(colour, position, in.clip_position.xy)), saturate(density * 0.8));
  }

  let travelled = in.extra.x;
  let down = travelled / max(in.extra.y, 0.01);
  let celsius = in.extra.z;
  let direction = normalize(in.flow + vec2<f32>(1.0e-5, 0.0));
  let side = vec2<f32>(-direction.y, direction.x);
  let u = dot(in.rest_xz, side);
  let lod = max(log2(footprint * 512.0 / 9.0), 0.0);
  // Streaks fall at the speed the water hits the pool, sqrt(2 g drop), a
  // constant for the whole sheet.
  let streak_uv = vec2<f32>(u / 1.3, (travelled - t * in.spacing) / 9.0);
  let s1 = textureSampleLevel(noise_texture, linear_sampler, streak_uv, lod).r;
  let s2 = textureSampleLevel(noise_texture, linear_sampler, streak_uv * vec2<f32>(2.3, 1.7) + vec2<f32>(0.37, 0.61), lod).g;
  let streak = s1 * 0.6 + s2 * 0.4;
  let edge = 1.0 - smoothstep(0.55, 1.0, abs(in.across));
  // The sheet breaks up into aerated white water as it falls.
  let aerated = smoothstep(0.15, 0.85, down);
  var alpha = saturate(mix(0.45, 0.92, aerated) * (0.55 + (streak - 0.35) * 1.4)) * mix(0.3, 1.0, edge);
  let normal = normalize(vec3<f32>(direction.x, 0.3, direction.y));
  let albedo = mix(vec3<f32>(0.5, 0.64, 0.68), vec3<f32>(0.92, 0.94, 0.95), max(aerated, streak * 0.5));
  let shadow = sun_visibility(position, normal);
  let lit = sun_light() * shadow * (saturate(dot(normal, sun)) * 0.8 + 0.2) + sky_irradiance(normal) * 0.6;
  // Sunlight shining through the thin sheet from behind.
  let back = pow(saturate(dot(-view, sun)), 4.0) * (1.0 - aerated * 0.6) * (1.0 - alpha * 0.5);
  var colour = albedo * lit / PI * 2.4 + sun_light() * shadow * vec3<f32>(0.5, 0.75, 0.7) * back * 0.4;

  if (freezing_possible()) {
    let ice = freeze_fraction(celsius, -8.0);

    if (ice > 0.001) {
      // An icefall: still, blue-white, ribbed down the fall line.
      let grain = textureSampleLevel(noise_texture, linear_sampler, vec2<f32>(u / 2.0, travelled / 7.0), lod).r;
      let rib = 0.5 + 0.5 * sin(u * 2.7 + grain * 5.0);
      let ice_albedo = mix(vec3<f32>(0.5, 0.68, 0.82), vec3<f32>(0.88, 0.93, 0.97), rib * 0.6 + grain * 0.4);
      let ice_normal = normalize(normal + vec3<f32>(side.x, 0.0, side.y) * (rib - 0.5) * 0.8);
      let ice_colour = shade_surface(ice_albedo, ice_normal, position, 1.0, 0.4, 0.35);
      colour = mix(colour, ice_colour, ice);
      alpha = mix(alpha, 0.97 * mix(0.5, 1.0, edge), ice);
    }
  }

  return vec4<f32>(finish_colour(apply_fog(colour, position, in.clip_position.xy)), saturate(alpha));
}

// Screen position of a world point: uv, then linear view depth.
fn screen_uv(p: vec3<f32>) -> vec3<f32> {
  let clip = frame.view_proj * vec4<f32>(p, 1.0);
  let ndc = clip.xy / max(clip.w, 1.0e-4);
  return vec3<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, clip.w);
}

// The scene copy's depth at a uv, unfiltered: filtering would blend a
// near edge with the sky behind it into a depth that is neither.
fn copy_depth(uv: vec2<f32>) -> f32 {
  let size = vec2<f32>(textureDimensions(scene_copy));
  return textureLoad(scene_copy, vec2<i32>(clamp(uv * size, vec2<f32>(0.0), size - 1.0)), 0).a;
}

// How much of a reflection hit to trust: none at the screen edges or at
// the end of the ray's reach, all of it in between.
fn reflection_fade(uv: vec2<f32>, travelled: f32, reach: f32) -> f32 {
  let edge = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
  return smoothstep(0.0, 0.1, edge) * (1.0 - smoothstep(0.6, 1.0, travelled / reach));
}

// The opaque scene seen along a reflected ray: 16 steps of geometrically
// growing stride from 2 m out to `reach` metres, so near banks and far
// hills are both found, then 4 bisection steps against the scene copy's
// depth. rgb is the colour, a how much to use it.
fn trace_reflection(origin: vec3<f32>, ray: vec3<f32>, reach: f32) -> vec4<f32> {
  var near_t = 0.0;
  var far_t = -1.0;

  for (var i = 1; i <= 16; i = i + 1) {
    let t = 2.0 * pow(reach / 2.0, f32(i) / 16.0);
    let s = screen_uv(origin + ray * t);

    if (s.z <= 0.0 || any(s.xy < vec2<f32>(0.0)) || any(s.xy > vec2<f32>(1.0))) {
      break;
    }

    if (s.z > copy_depth(s.xy)) {
      far_t = t;
      break;
    }

    near_t = t;
  }

  if (far_t < 0.0) {
    return vec4<f32>(0.0);
  }

  for (var k = 0; k < 4; k = k + 1) {
    let t = 0.5 * (near_t + far_t);
    let s = screen_uv(origin + ray * t);

    if (s.z > copy_depth(s.xy)) {
      far_t = t;
    } else {
      near_t = t;
    }
  }

  let s = screen_uv(origin + ray * far_t);
  let hit = vec4<f32>(textureSampleLevel(scene_copy, linear_sampler, s.xy, 0.0).rgb, copy_depth(s.xy));
  // A ray passing far behind what it crossed found nothing there, and one
  // heading back towards the camera sees surfaces facing away. Nothing
  // nearer the camera than the water itself can be in its reflection:
  // that is foreground in front of it.
  let solid = (1.0 - smoothstep(0.1, 0.25, (s.z - hit.a) / max(s.z, 1.0)))
    * step(0.95 * screen_uv(origin).z, hit.a);
  let facing = smoothstep(-0.1, 0.2, dot(ray, frame.camera_forward.xyz));
  return vec4<f32>(hit.rgb, reflection_fade(s.xy, far_t, reach) * solid * facing);
}

// The stone in the cell of a grid `size` metres apart around `p`, if the
// cell has one (with probability `chance`): xy its centre, z its radius
// (from `radius.x` to `radius.y`), or 0 without one. Each stone stays
// inside its cell, so one cell is all a pixel needs.
fn bed_stone(p: vec2<f32>, size: f32, radius: vec2<f32>, chance: f32) -> vec3<f32> {
  let cell = floor(p / size) + size;
  let r = mix(radius.x, radius.y, hash12(cell + 7.3));
  let jitter = vec2<f32>(hash12(cell + 1.7), hash12(cell + 4.1)) - 0.5;
  let centre = (cell - size + 0.5) * size + jitter * (size - 2.0 * r);
  return vec3<f32>(centre, select(0.0, r, hash12(cell) < chance));
}

@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
  let t = time_seconds();
  // How many metres one pixel covers, for the level of the texture
  // samples inside branches below.
  let footprint = length(frame.camera_position.xyz - in.world_position) * frame.camera_forward.w * 2.0 * frame.viewport.w;
  let position = in.world_position;
  let camera = frame.camera_position.xyz;
  let view_offset = camera - position;
  let distance = length(view_offset);
  let view = view_offset / max(distance, 0.001);
  let bed = terrain_height_at(in.rest_xz);
  let depth = max(position.y - bed, 0.0);
  let ripple_strength = frame.water_params.x;
  let current = frame.water_current.xy;

  // Detail ripples: two scrolling layers at different scales and
  // directions so the pattern never visibly repeats or slides as one.
  let wind_a = current + vec2<f32>(t * 0.9, t * 0.35);
  let wind_b = current * 0.7 + vec2<f32>(-t * 0.4, t * 0.8);
  let uv_a = (in.rest_xz + wind_a) / 11.0;
  let uv_b = (in.rest_xz + wind_b) / 29.0 + vec2<f32>(0.37, 0.19);
  var detail = detail_normal(uv_a) * 0.6 + detail_normal(uv_b) * 0.4;

  // River current: two-phase flow map advecting the ripple texture along
  // the channel, cross-faded so the stretch never becomes visible. Fast
  // water has smaller, choppier ripples; slow water is nearly glassy.
  // Snowmelt speeds it up, and the outer bank of a bend runs faster.
  let river = INLAND && in.kind == KIND_RIVER;
  let ocean = !INLAND && in.kind == KIND_OCEAN;
  let melt = select(1.0, frame.rivers.x, river);
  let slope = select(0.0, in.extra.x, river);
  let curvature = select(0.0, in.extra.y, river);
  let outer = saturate(-sign(curvature) * in.across) * abs(curvature);
  let flow = in.flow * melt;
  let flow_speed = length(flow);
  let ripple_size = mix(7.0, 3.5, smoothstep(0.5, 2.0, flow_speed));
  let flow_phase_0 = fract(t * 0.35);
  let flow_phase_1 = fract(t * 0.35 + 0.5);
  let flow_uv = in.rest_xz / ripple_size;
  // One flow-map cycle lasts 1 / 0.35 s, so the texture moves with the
  // water.
  let advect = flow * (1.0 + outer * 0.35) / (0.35 * ripple_size);
  let flow_a = detail_normal(flow_uv - advect * flow_phase_0);
  let flow_b = detail_normal(flow_uv - advect * flow_phase_1 + vec2<f32>(0.5, 0.5));
  let flow_blend = abs(flow_phase_0 * 2.0 - 1.0);
  let foam_texel = textureSample(water_texture, linear_sampler, (in.rest_xz - flow * t * 0.6) / 13.0).b;
  let shore_foam_texel = textureSample(water_texture, linear_sampler, (in.rest_xz + current * 1.3) / 9.0 + vec2<f32>(t * 0.02, 0.0)).b;
  // Waterfalls and mist, after every implicit-derivative sample: they
  // branch per primitive.

  // Rapids: on slopes of 2 to 8 %, and below small steps.
  let rapids = smoothstep(0.015, 0.025, slope);
  // Unit stream power (rho g Q S / w = rho g v d S): over 300 W/m² on
  // slopes over 2 % the banks are rock, the stones big and the rapids
  // wild (see `rock_banks` in `terrain/channels.rs`).
  let power = 9810.0 * flow_speed * in.extra.z * slope;
  let wild = select(0.0, smoothstep(300.0, 600.0, power) * smoothstep(0.02, 0.03, slope), river);

  if (river) {
    detail = mix(flow_a, flow_b, flow_blend) * mix(0.12, 1.2, smoothstep(0.5, 2.0, flow_speed));

    // Standing waves: crests across the current, fixed in space, about
    // 2 pi v^2 / g apart.
    if (rapids > 0.001) {
      let along = normalize(flow + vec2<f32>(1.0e-5, 0.0));
      let spacing = max(TAU * flow_speed * flow_speed / 9.81, 0.8);
      let wobble = textureSampleLevel(noise_texture, linear_sampler, in.rest_xz / 23.0, texture_lod(footprint, 23.0)).r;
      let crest = cos((dot(in.rest_xz, along) / spacing + wobble * 3.0) * TAU);
      detail = detail + vec3<f32>(along.x, 0.0, along.y) * crest * rapids * 0.45 * (1.0 + wild);
    }
  }

  // Plunge pools: churned water flowing out from the foot of the fall.
  var churn = 0.0;

  if (INLAND && falls_possible() && in.kind == KIND_POOL) {
    let r = in.across;
    let outward = normalize(in.flow + vec2<f32>(1.0e-5, 0.0));
    let energy = saturate(sqrt(max(in.extra.y, 0.0)) * 0.25 + 0.3);
    let lod = texture_lod(footprint, 5.0);
    let pool_a = textureSampleLevel(water_texture, linear_sampler, in.rest_xz / 5.0 - outward * flow_phase_0 * 1.5, lod);
    let pool_b = textureSampleLevel(water_texture, linear_sampler, in.rest_xz / 5.0 - outward * flow_phase_1 * 1.5 + vec2<f32>(0.5, 0.5), lod);
    let pool = mix(pool_a, pool_b, flow_blend);
    detail = vec3<f32>(pool.r * 2.0 - 1.0, 0.0, pool.g * 2.0 - 1.0) * (0.6 + energy);
    // Rings of foam spread from the foot and fade as they go.
    let rings = pow(1.0 - fract(r * 2.2 - t * 0.45), 4.0) * (1.0 - smoothstep(0.5, 1.0, r));
    churn = saturate((rings * 0.7 + (1.0 - smoothstep(0.0, 0.45, r))) * pool.b * 1.6) * energy;

    if (freezing_possible()) {
      churn = churn * (1.0 - freeze_fraction(in.extra.w, -8.0));
    }
  }

  // Raindrops: expanding rings in a jittered grid, each cell restarting at
  // its own random time.
  let rain = frame.weather.x;

  if (rain > 0.01 && distance < 120.0) {
    let cell_size = 0.9;
    let cell = floor(in.rest_xz / cell_size);
    let local = in.rest_xz / cell_size - cell;
    let seed = hash12(cell);
    let centre = vec2<f32>(hash12(cell + 17.1), hash12(cell + 3.7)) * 0.6 + 0.2;
    let age = fract(t * (0.8 + seed * 0.6) + seed);
    let offset = local - centre;
    let radius = length(offset);
    let ring = sin((radius - age * 0.5) * 48.0) * (1.0 - age) * smoothstep(0.5, 0.0, radius)
      * step(radius, age * 0.5 + 0.05);
    detail = detail + vec3<f32>(offset.x, 0.0, offset.y) / max(radius, 0.001) * ring * rain * 1.5
      * (1.0 - distance / 120.0);
  }

  var normal = vec3<f32>(0.0, 1.0, 0.0);
  var jacobian = 1.0;
  var crest = 0.0;
  let pixel_footprint = distance * frame.camera_forward.w * 2.0 * frame.viewport.w;
  var ice = 0.0;
  var floes = Floes(0.0, 0.0, 0.0, vec2<f32>(0.0, 1.0), 0.0, vec2<f32>(0.0, 1.0), 0.0, 0.0, 0.5);
  // How much of the pixel is floes or slush. Where it is all ice, the open
  // water beneath is never seen, so it is not shaded.
  var solid = 0.0;

  if (ocean && sea_ice_possible()) {
    ice = sea_ice_concentration(in.rest_xz);

    if (ice > 0.001) {
      floes = pack_ice(in.rest_xz + frame.sea_ice.zw, ice, pixel_footprint, distance);
      solid = floes.cover + (1.0 - floes.cover) * floes.brash;
    }
  }

  if (ocean && solid < 0.999) {
    let waves = sample_waves(in.rest_xz, depth, max(in.spacing, pixel_footprint * 2.0));
    // Waves die down in the water between floes.
    normal = normalize(mix(waves.normal, vec3<f32>(0.0, 1.0, 0.0), ice));
    jacobian = mix(waves.jacobian, 1.0, ice);
    crest = waves.height / max(frame.wave_params.x * 0.5, 0.01) * (1.0 - ice);
    detail = detail * (1.0 - ice * 0.8);
  }

  let sun = sun_dir();
  var colour = vec3<f32>(0.0);
  var alpha_out = 1.0;

  if (solid < 0.999) {
    // Ripples fade with distance so far water turns into a calm mirror
    // instead of aliasing.
    let detail_fade = 1.0 - smoothstep(60.0, 1400.0, distance);
    normal = normalize(normal + detail * ripple_strength * 0.55 * (0.25 + 0.75 * detail_fade));

    let n_dot_v = saturate(dot(normal, view));
    let fresnel = 0.02 + 0.98 * pow(1.0 - n_dot_v, 5.0);
    var reflected = reflect(-view, normal);
    reflected.y = abs(reflected.y);
    var reflection = sky_radiance(reflected);

    // Clouds reflected on the water.
    if (frame.cloud_params.x > 0.001 && reflected.y > 0.02) {
      let travel = max(frame.cloud_params.y - position.y, 10.0) / reflected.y;
      let weather = cloud_weather(position.xz + reflected.xz * travel);
      // Under a full overcast the deck already is the sky being reflected.
      let cloud = smoothstep(0.05, 0.6, weather) * saturate(reflected.y * 5.0) * (1.0 - frame.weather2.y);
      reflection = mix(reflection, (sun_light() * 0.35 + sky_irradiance(vec3<f32>(0.0, 1.0, 0.0)) * 0.3) * frame.cloud_colour.rgb, cloud * 0.8);
    }

    // The scene on screen where a reflected ray finds it, over the sky.
    if (frame.water_origin.w > 0.5) {
      let hit = trace_reflection(position, reflected, 4000.0);
      reflection = mix(reflection, hit.rgb, hit.a);
    }

    let shadow = sun_visibility(position, vec3<f32>(0.0, 1.0, 0.0));

    // Sun glitter: GGX with roughness that grows with distance.
    let roughness = mix(0.05, 0.22, smoothstep(50.0, 3000.0, distance)) + ripple_strength * 0.02;
    let half_vector = normalize(view + sun);
    let n_dot_h = saturate(dot(normal, half_vector));
    let alpha = roughness * roughness;
    let ggx = alpha * alpha / (PI * pow(n_dot_h * n_dot_h * (alpha * alpha - 1.0) + 1.0, 2.0));
    // The sun's glint needs the sun's disc; an overcast deck hides it.
    let glint = (1.0 - frame.weather2.y) * (1.0 - frame.weather2.y);
    let specular = sun_light() * shadow * ggx * fresnel * saturate(dot(normal, sun)) * 0.9 * glint;

    // Water body: absorption by depth, lit by sky and sun.
    // Fast rivers carry silt, which clouds and browns them.
    let sediment = select(0.0, saturate((flow_speed - 1.0) / 3.0), river);
    let clarity = max(frame.water_params.z, 0.1) * (1.0 - sediment * 0.6);
    let absorption = 1.0 - exp(-depth / clarity);
    let body_colour = mix(
      mix(srgb_to_linear(frame.water_shallow.rgb), srgb_to_linear(frame.water_deep.rgb), absorption),
      vec3<f32>(0.2, 0.17, 0.09),
      sediment * 0.55
    );
    let body_light = sky_irradiance(vec3<f32>(0.0, 1.0, 0.0)) * 0.35 + sun_light() * shadow * (0.35 + 0.65 * saturate(sun.y));
    // Leads between floes are dark: the ice shades the water beneath.
    var body = body_colour * body_light * 0.55 * (1.0 - ice * 0.5);

    // Light shining through the thin tops of waves.
    let subsurface = pow(saturate(dot(view, -sun) * 0.5 + 0.5), 3.0) * saturate(crest) * 0.9;
    body = body + srgb_to_linear(frame.water_shallow.rgb) * sun_light() * shadow * subsurface * 0.35;

    let opacity = saturate(1.0 - exp(-depth / (clarity * 0.45)) + 0.08);
    let reflectivity = clamp(0.55 + frame.water_params.y * 0.9, 0.0, 1.0);
    let reflect_weight = fresnel * reflectivity;
    alpha_out = reflect_weight + opacity * (1.0 - reflect_weight);
    colour = (reflection * reflect_weight + body * opacity * (1.0 - reflect_weight)) / max(alpha_out, 0.001) + specular;

    // Foam: breaking crests, shorelines (bands that roll in towards the
    // beach), and fast-flowing rapids.
    let foam_strength = frame.water_params.w;
    var foam = 0.0;

    if (ocean) {
      let breaking = smoothstep(0.55, 0.15, jacobian) * foam_texel;
      let swell = max(frame.wave_params.x, 0.2);
      let surf_zone = 1.0 - smoothstep(0.0, 1.2 + swell * 1.5, depth);
      let bands = pow(saturate(sin(depth * 2.2 - t * 1.6) * 0.5 + 0.5), 3.0);
      foam = max(breaking, surf_zone * mix(0.35, 1.0, bands) * shore_foam_texel * 1.6);
    } else if (river) {
      // Whitewater on rapids, and on the outer bank of bends.
      let whitewater = saturate((flow_speed - 1.5) / 2.5) * rapids * (1.0 + wild);
      foam = saturate(whitewater * foam_texel * 1.6 + outer * 0.5 * foam_texel) * melt;
    } else {
      let edge = 1.0 - smoothstep(0.0, 0.8, depth);
      foam = max(edge * shore_foam_texel * 0.4, churn);
    }

    // Stones on the bed of shallow, fast water, 0.3 to 0.8 m across, or
    // 0.8 to 2 m in wild water: those breaking the surface are drawn as
    // wet stones with foam on their upstream side and, in wild water,
    // streaks downstream; those just under it make a bright riffle.
    var stone = 0.0;
    var stone_normal = vec3<f32>(0.0, 1.0, 0.0);
    var stone_shade = 0.0;
    let shallow = min(depth, in.extra.z);

    if (river && flow_speed > 0.5 && shallow < 1.0 && distance < 150.0) {
      let small = bed_stone(in.rest_xz, 2.0, vec2<f32>(0.15, 0.4), 0.5 * (1.0 - wild));
      let large = bed_stone(in.rest_xz, 3.5, vec2<f32>(0.4, 1.0), 0.75 * wild);
      let pick = select(small, large, large.z > 0.0);

      if (pick.z > 0.0) {
        let offset = (in.rest_xz - pick.xy) / pick.z;
        let d = length(offset);
        let downstream = normalize(flow + vec2<f32>(1.0e-5, 0.0));
        let along = dot(offset, downstream);
        let dome = sqrt(max(1.0 - d * d, 0.0));
        let top = 1.1 * pick.z;
        let emerge = top * dome - shallow;
        let breaks = step(shallow, top);
        let ring = (1.0 - smoothstep(1.0, 1.4, d)) * step(1.0, d) * smoothstep(0.0, -0.5, along) * breaks;
        let streak = wild * breaks * step(0.0, along) * (1.0 - smoothstep(0.0, 7.0, along))
          * (1.0 - smoothstep(0.3, 0.8, abs(offset.x * downstream.y - offset.y * downstream.x))) * foam_texel;
        let riffle = step(d, 1.0) * smoothstep(-0.1, 0.0, emerge) * (1.0 - step(0.0, emerge)) * foam_texel * 0.7;
        foam = max(foam, max(ring, max(streak, riffle)) * melt);
        stone = step(d, 1.0) * step(0.0, emerge);
        stone_normal = normalize(vec3<f32>(offset.x, dome + 0.3, offset.y));
        stone_shade = hash12(pick.xy);
      }
    }

    foam = saturate(foam * foam_strength * (1.0 - smoothstep(400.0, 2500.0, distance) * 0.7)) * (1.0 - ice);
    let foam_colour = vec3<f32>(0.9) * (sun_light() * shadow * saturate(sun.y + 0.2) + sky_irradiance(vec3<f32>(0.0, 1.0, 0.0)) * 0.4) / PI * 2.2;
    colour = mix(colour, foam_colour, foam);
    alpha_out = max(alpha_out, foam);

    // Fade out the thinnest film of water at the waterline. Rivers,
    // lakes and pools reach past their banks, so the bank itself hides
    // where they end.
    if (ocean) {
      alpha_out = alpha_out * smoothstep(0.0, 0.12, depth + 0.1);
    } else {
      alpha_out = alpha_out * smoothstep(0.0, 0.25, depth);
    }

    // A soft edge where a river's ribbon ends over lower ground.
    if (river) {
      alpha_out = alpha_out * (1.0 - smoothstep(0.7, 1.0, abs(in.across))) * in.coverage;
    }

    // Stones stand out of the water even where the film at the edge
    // fades.
    if (stone > 0.0) {
      let albedo = mix(vec3<f32>(0.2, 0.19, 0.18), vec3<f32>(0.25, 0.2, 0.15), stone_shade) * (0.7 + stone_shade * 0.5);
      colour = shade_surface(albedo, stone_normal, position, 1.0, 0.9, 0.15);
      alpha_out = 1.0 - smoothstep(0.85, 1.0, abs(in.across));
    }

    if (INLAND && freezing_possible() && (in.kind == KIND_LAKE || river || in.kind == KIND_POOL)) {
      let frozen = frozen_surface(in, position, depth, flow_speed, pixel_footprint, distance);
      colour = mix(colour, frozen.rgb, frozen.a);
      alpha_out = mix(alpha_out, smoothstep(0.0, 0.25, depth), frozen.a);
    }

    // A pool holds water only in its bowl, `extra.x` deep at the centre
    // and rising to the rim: where the ground drops away below the bowl (a
    // pool smaller than a heightmap sample, on a slope) the water would
    // have drained, so it fades out, and it fades towards the rim, frozen
    // or not.
    if (INLAND && in.kind == KIND_POOL) {
      let bowl = in.extra.x * (1.0 - in.across * in.across);
      alpha_out = alpha_out * (1.0 - smoothstep(bowl + 0.3, bowl + 1.0, depth))
        * (1.0 - smoothstep(0.6, 1.0, in.across));
    }
  }

  // Floes over grey slush, so their soft edges blend into slush, never
  // into a line of open water. Both are shaded in one go.
  if (solid > 0.001) {
    let share = floes.cover / solid;
    // Snow-covered floes, some bare and blue-grey under thin snow, with
    // rounded, bevelled rims and bright pressure ridges.
    let tilt = (floes.outward * floes.rim * 0.55 - floes.ridge_side * floes.ridge * 0.6) * share;
    let ice_normal = normalize(vec3<f32>(tilt.x, 1.0, tilt.y));
    var albedo = mix(vec3<f32>(0.8, 0.84, 0.9) * (0.74 + floes.shade * 0.12 + floes.grain * 0.2), vec3<f32>(0.62, 0.72, 0.78) * 0.8, floes.bare);
    albedo = mix(albedo, vec3<f32>(0.9, 0.93, 0.97), floes.ridge);
    // Light scattered in the snow comes out blue on the shadowed side.
    albedo = albedo * mix(vec3<f32>(1.0), vec3<f32>(0.86, 0.93, 1.06), (1.0 - saturate(dot(ice_normal, sun))) * 0.6);
    // Slush is matt and grey, with no glint.
    albedo = mix(vec3<f32>(0.4, 0.43, 0.46), albedo, share);
    let ice_colour = shade_surface(albedo, ice_normal, position, 1.0 - floes.rim * 0.25 * share, (0.3 + floes.bare * 0.3) * share, mix(0.9, 0.45, share));
    colour = mix(colour, ice_colour, solid);
    alpha_out = mix(alpha_out, 1.0, solid);
  }

  colour = apply_fog(colour, position, in.clip_position.xy);
  // Rain and snow fall in front of the water too.
  let falling = precipitation(-view, distance);
  colour = mix(colour, falling.rgb, falling.a);
  return vec4<f32>(finish_colour(colour), saturate(alpha_out));
}
