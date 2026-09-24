// Renders the ocean, rivers, and lakes. `common.wgsl` is prepended.
//
// Ocean: a camera-following grid displaced by a sum of eight Gerstner
// waves spread around the configured swell direction. Waves shoal (shrink,
// then break into foam) as the water gets shallow, and each wave fades out
// where the grid is too coarse to represent it, so distant water never
// aliases. Rivers: ribbons whose vertices carry the local current; the
// detail normal map is advected along it with a two-phase flow map, so
// water visibly runs downstream, faster on steep reaches. Lakes: flat
// surfaces rippled by the wind-driven current.
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
};

struct VertexOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) world_position: vec3<f32>,
  @location(1) flow: vec2<f32>,
  @location(2) @interpolate(flat) kind: i32,
  @location(3) across: f32,
  @location(4) spacing: f32,
  @location(5) rest_xz: vec2<f32>,
};

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
fn pack_ice(xz: vec2<f32>, concentration: f32, footprint: f32, distance: f32) -> Floes {
  let p = xz + frame.sea_ice.zw;
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

  if (kind == 0) {
    position = vec3<f32>(in.position.x + frame.water_origin.x, frame.water_shallow.w, in.position.z + frame.water_origin.y);
  }

  var out: VertexOut;
  out.rest_xz = position.xz;

  if (kind == 0) {
    let depth = position.y - terrain_height_at(position.xz);
    let waves = sample_waves(position.xz, depth, in.params.z);
    var displacement = waves.displacement;

    // Pack ice damps the swell.
    if (sea_ice_possible()) {
      displacement = displacement * (1.0 - sea_ice_concentration(position.xz));
    }

    position = position + displacement;
  }

  out.clip_position = frame.view_proj * vec4<f32>(position, 1.0);
  out.world_position = position;
  out.flow = in.flow;
  out.kind = kind;
  out.across = in.params.y;
  out.spacing = in.params.z;
  return out;
}

fn detail_normal(uv: vec2<f32>) -> vec3<f32> {
  let texel = textureSample(water_texture, linear_sampler, uv);
  return vec3<f32>(texel.r * 2.0 - 1.0, 0.0, texel.g * 2.0 - 1.0);
}

@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
  let t = time_seconds();
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
  // the channel, cross-faded so the stretch never becomes visible.
  let flow_speed = length(in.flow);
  let flow_phase_0 = fract(t * 0.35);
  let flow_phase_1 = fract(t * 0.35 + 0.5);
  let flow_uv = in.rest_xz / 7.0;
  let flow_a = detail_normal(flow_uv - in.flow * flow_phase_0 * 1.2);
  let flow_b = detail_normal(flow_uv - in.flow * flow_phase_1 * 1.2 + vec2<f32>(0.5, 0.5));
  let flow_blend = abs(flow_phase_0 * 2.0 - 1.0);
  let foam_texel = textureSample(water_texture, linear_sampler, (in.rest_xz - in.flow * t * 0.6) / 13.0).b;
  let shore_foam_texel = textureSample(water_texture, linear_sampler, (in.rest_xz + current * 1.3) / 9.0 + vec2<f32>(t * 0.02, 0.0)).b;

  if (in.kind == 1) {
    detail = mix(flow_a, flow_b, flow_blend) * (0.6 + min(flow_speed, 3.0) * 0.35);
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

  if (in.kind == 0 && sea_ice_possible()) {
    ice = sea_ice_concentration(in.rest_xz);

    if (ice > 0.001) {
      floes = pack_ice(in.rest_xz, ice, pixel_footprint, distance);
      solid = floes.cover + (1.0 - floes.cover) * floes.brash;
    }
  }

  if (in.kind == 0 && solid < 0.999) {
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
    let clarity = max(frame.water_params.z, 0.1);
    let absorption = 1.0 - exp(-depth / clarity);
    let body_colour = mix(srgb_to_linear(frame.water_shallow.rgb), srgb_to_linear(frame.water_deep.rgb), absorption);
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

    if (in.kind == 0) {
      let breaking = smoothstep(0.55, 0.15, jacobian) * foam_texel;
      let swell = max(frame.wave_params.x, 0.2);
      let surf_zone = 1.0 - smoothstep(0.0, 1.2 + swell * 1.5, depth);
      let bands = pow(saturate(sin(depth * 2.2 - t * 1.6) * 0.5 + 0.5), 3.0);
      foam = max(breaking, surf_zone * mix(0.35, 1.0, bands) * shore_foam_texel * 1.6);
    } else {
      let edge = 1.0 - smoothstep(0.0, 0.8, depth);
      foam = max(edge * shore_foam_texel * 0.8, smoothstep(1.6, 3.5, flow_speed) * foam_texel);
    }

    foam = saturate(foam * foam_strength * (1.0 - smoothstep(400.0, 2500.0, distance) * 0.7)) * (1.0 - ice);
    let foam_colour = vec3<f32>(0.9) * (sun_light() * shadow * saturate(sun.y + 0.2) + sky_irradiance(vec3<f32>(0.0, 1.0, 0.0)) * 0.4) / PI * 2.2;
    colour = mix(colour, foam_colour, foam);
    alpha_out = max(alpha_out, foam);

    // Soften river ribbon edges where they meet the bank.
    if (in.kind == 1) {
      alpha_out = alpha_out * (1.0 - smoothstep(0.75, 1.0, abs(in.across)));
    }

    // Fade out the thinnest film of water at the waterline.
    alpha_out = alpha_out * smoothstep(0.0, 0.12, depth + select(0.0, 0.1, in.kind == 0));
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
