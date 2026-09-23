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
    position = position + waves.displacement;
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

  if (in.kind == 0) {
    let pixel_footprint = distance * frame.camera_forward.w * 2.0 * frame.viewport.w;
    let waves = sample_waves(in.rest_xz, depth, max(in.spacing, pixel_footprint * 2.0));
    normal = waves.normal;
    jacobian = waves.jacobian;
    crest = waves.height / max(frame.wave_params.x * 0.5, 0.01);
  }

  // Ripples fade with distance so far water turns into a calm mirror
  // instead of aliasing.
  let detail_fade = 1.0 - smoothstep(60.0, 1400.0, distance);
  normal = normalize(normal + detail * ripple_strength * 0.55 * (0.25 + 0.75 * detail_fade));

  let sun = sun_dir();
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
  var body = body_colour * body_light * 0.55;

  // Light shining through the thin tops of waves.
  let subsurface = pow(saturate(dot(view, -sun) * 0.5 + 0.5), 3.0) * saturate(crest) * 0.9;
  body = body + srgb_to_linear(frame.water_shallow.rgb) * sun_light() * shadow * subsurface * 0.35;

  let opacity = saturate(1.0 - exp(-depth / (clarity * 0.45)) + 0.08);
  let reflectivity = clamp(0.55 + frame.water_params.y * 0.9, 0.0, 1.0);
  let reflect_weight = fresnel * reflectivity;
  var alpha_out = reflect_weight + opacity * (1.0 - reflect_weight);
  var colour = (reflection * reflect_weight + body * opacity * (1.0 - reflect_weight)) / max(alpha_out, 0.001) + specular;

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

  foam = saturate(foam * foam_strength * (1.0 - smoothstep(400.0, 2500.0, distance) * 0.7));
  let foam_colour = vec3<f32>(0.9) * (sun_light() * shadow * saturate(sun.y + 0.2) + sky_irradiance(vec3<f32>(0.0, 1.0, 0.0)) * 0.4) / PI * 2.2;
  colour = mix(colour, foam_colour, foam);
  alpha_out = max(alpha_out, foam);

  // Soften river ribbon edges where they meet the bank.
  if (in.kind == 1) {
    alpha_out = alpha_out * (1.0 - smoothstep(0.75, 1.0, abs(in.across)));
  }

  // Fade out the thinnest film of water at the waterline.
  alpha_out = alpha_out * smoothstep(0.0, 0.12, depth + select(0.0, 0.1, in.kind == 0));

  colour = apply_fog(colour, position, in.clip_position.xy);
  // Rain and snow fall in front of the water too.
  let falling = precipitation(-view, distance);
  colour = mix(colour, falling.rgb, falling.a);
  return vec4<f32>(finish_colour(colour), saturate(alpha_out));
}
