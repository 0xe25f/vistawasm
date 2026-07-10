// Renders an analytic sky dome as a full-screen background pass drawn
// before terrain, flora, and water. It approximates Rayleigh scattering
// (blue sky brightening towards the zenith), Mie forward scattering (haze
// and glow around the sun), a sun disc, horizon haze, and an optional
// cloud layer, driven by the public atmosphere/cloud/sun controls. This is
// a practical approximation, not a physically integrated scattering
// simulation.
//
// Clouds have two styles (see `CloudsOptions` in `vista_types` and
// `docs/environment-upgrade-plan.md` §4): `Painted` samples a single 2-D
// noise layer at the cloud altitude (cheap, the default once enabled);
// `Volumetric` raymarches a thin density band around that altitude for
// real depth and sun-facing shading. Both are folded into this one
// fullscreen pass rather than a separate pipeline — the pass already
// reconstructs a per-pixel view ray, and terrain drawn afterwards already
// occludes distant sky/cloud pixels behind mountains via the depth buffer,
// so no extra pipeline or depth-awareness is needed here.

struct FrameUniforms {
  view_proj: mat4x4<f32>,
  camera_position: vec4<f32>,
  sun_direction: vec4<f32>,
  sun_colour_intensity: vec4<f32>,
  fog: vec4<f32>,
  water_params: vec4<f32>,
  camera_forward: vec4<f32>,
  camera_right: vec4<f32>,
  camera_up: vec4<f32>,
  camera_params: vec4<f32>,
  sky_tint: vec4<f32>,
  atmosphere_params: vec4<f32>,
  mist_params: vec4<f32>,
  mist_colour: vec4<f32>,
  cloud_params: vec4<f32>,
  cloud_colour: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> frame: FrameUniforms;

struct VertexOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) ndc: vec2<f32>,
};

@vertex
fn vertex_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
  var positions = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(3.0, -1.0),
    vec2<f32>(-1.0, 3.0)
  );
  let position = positions[vertex_index];

  var out: VertexOut;
  out.clip_position = vec4<f32>(position, 0.0, 1.0);
  out.ndc = position;
  return out;
}

fn hash21(p: vec2<f32>) -> f32 {
  var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
  p3 = p3 + dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

fn cloud_value_noise(p: vec2<f32>) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let a = hash21(i);
  let b = hash21(i + vec2<f32>(1.0, 0.0));
  let c = hash21(i + vec2<f32>(0.0, 1.0));
  let d = hash21(i + vec2<f32>(1.0, 1.0));
  let u = f * f * (3.0 - 2.0 * f);
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn cloud_fbm(p: vec2<f32>) -> f32 {
  var value = 0.0;
  var amplitude = 0.55;
  var frequency = 1.0;

  for (var i = 0; i < 4; i = i + 1) {
    value = value + cloud_value_noise(p * frequency) * amplitude;
    amplitude = amplitude * 0.5;
    frequency = frequency * 2.05;
  }

  return value;
}

// Returns cloud density (0..1) for a view ray, using the cheap `Painted`
// single-sample style or the raymarched `Volumetric` style depending on
// `frame.cloud_params.w` (0 selects Painted; a validated 8..=64 step count
// selects Volumetric — see `config.rs`'s `validate_clouds`).
fn cloud_density(ray: vec3<f32>, drift: vec2<f32>) -> f32 {
  let coverage = frame.cloud_params.x;
  let cloud_height = frame.cloud_params.z;
  let raymarch_steps = frame.cloud_params.w;
  let height_above_camera = max(cloud_height - frame.camera_position.y, 50.0);
  let travel = height_above_camera / max(ray.y, 0.02);
  let sample_xz = vec2<f32>(frame.camera_position.x, frame.camera_position.z)
    + vec2<f32>(ray.x, ray.z) * travel + drift;

  if (raymarch_steps < 0.5) {
    let noise = cloud_fbm(sample_xz * 0.00025);
    return clamp((noise + coverage - 0.5) * 2.2, 0.0, 1.0);
  }

  // Accumulate as `1 - transmittance` (a standard front-to-back opacity
  // composite) rather than a plain sum of per-step densities, so the
  // result always stays within 0..1 regardless of step count — a plain
  // sum over many steps would saturate to a uniform overcast well before
  // `coverage` reached 1, which is what an earlier version of this
  // function did.
  let steps = i32(raymarch_steps);
  let band_half_thickness = max(cloud_height * 0.08, 150.0);
  let step_length = (band_half_thickness * 2.0) / f32(steps);
  var transmittance = 1.0;
  var height_offset = -band_half_thickness;

  for (var step = 0; step < steps; step = step + 1) {
    let sample_travel = (cloud_height + height_offset - frame.camera_position.y) / max(ray.y, 0.02);
    let sample_pos = vec2<f32>(frame.camera_position.x, frame.camera_position.z)
      + vec2<f32>(ray.x, ray.z) * sample_travel + drift;
    let noise = cloud_fbm(sample_pos * 0.00025 + f32(step) * 0.013);
    let local_density = clamp((noise + coverage - 0.5) * 1.8, 0.0, 1.0);
    let extinction = local_density * (2.5 / f32(steps));
    transmittance = transmittance * clamp(1.0 - extinction, 0.0, 1.0);
    height_offset = height_offset + step_length;
  }

  return clamp(1.0 - transmittance, 0.0, 1.0);
}

@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
  let tan_half_fov_y = max(frame.camera_params.x, 0.0001);
  let aspect = max(frame.camera_params.y, 0.0001);
  let tan_half_fov_x = tan_half_fov_y * aspect;

  let ray = normalize(
    frame.camera_forward.xyz
      + frame.camera_right.xyz * in.ndc.x * tan_half_fov_x
      + frame.camera_up.xyz * in.ndc.y * tan_half_fov_y
  );

  let sun_direction = normalize(frame.sun_direction.xyz);
  let rayleigh_strength = frame.atmosphere_params.x;
  let mie_strength = frame.atmosphere_params.y;
  let sky_tint = frame.sky_tint.rgb;
  let exposure = frame.fog.y;

  // Rayleigh: a blue gradient that deepens towards the zenith and pales
  // towards the horizon.
  let elevation = clamp(ray.y, -1.0, 1.0);
  let zenith_factor = clamp(elevation, 0.0, 1.0);
  let rayleigh_colour = mix(
    vec3<f32>(0.86, 0.90, 0.92),
    vec3<f32>(0.10, 0.32, 0.70),
    pow(zenith_factor, 0.45)
  );

  // Mie: forward scattering haze around the sun, using a Henyey-Greenstein
  // style phase approximation.
  let cos_theta = dot(ray, sun_direction);
  let g = 0.82;
  let mie_phase = (1.0 - g * g) / pow(max(1.0 + g * g - 2.0 * g * cos_theta, 0.0001), 1.5);
  let mie_colour = vec3<f32>(1.0, 0.92, 0.78) * mie_phase * mie_strength * 0.06;

  // A bright sun disc where the view ray closely aligns with the sun.
  let sun_disc = smoothstep(0.9994, 0.9998, cos_theta);
  let sun_colour = frame.sun_colour_intensity.rgb * frame.sun_colour_intensity.w * sun_disc;

  // Horizon haze approximates aerial perspective for a sky that has no
  // terrain in view, blending towards the sky tint near the horizon.
  let horizon_haze = 1.0 - clamp(abs(elevation) * 3.0, 0.0, 1.0);
  let haze_colour = sky_tint * horizon_haze * clamp(mie_strength, 0.0, 2.0) * 0.5;

  var colour = rayleigh_colour * rayleigh_strength + mie_colour + haze_colour;
  colour = colour * sky_tint;
  colour = colour + sun_colour;

  // Clouds: skipped entirely (cheaply, one branch) when coverage is 0,
  // which is the default `Off` style's exact behaviour today.
  let coverage = frame.cloud_params.x;

  if (coverage > 0.0001 && ray.y > 0.02) {
    let drift = vec2<f32>(1.0, 0.6) * frame.water_params.z * frame.cloud_params.y * 40.0;
    let density = cloud_density(ray, drift);
    let sun_facing = clamp(dot(ray, sun_direction) * 0.5 + 0.5, 0.0, 1.0);
    let cloud_shade = mix(0.55, 1.15, sun_facing);
    let cloud_rgb = frame.cloud_colour.rgb * cloud_shade * frame.sun_colour_intensity.w;
    let horizon_fade = clamp(ray.y * 4.0, 0.0, 1.0);
    colour = mix(colour, cloud_rgb, density * horizon_fade);
  }

  colour = colour * exposure;

  // Darken the sky slightly below the horizon so an unbounded view does not
  // look like an infinite bright plane where terrain does not cover it.
  let below_horizon = clamp(-elevation, 0.0, 1.0);
  colour = mix(colour, colour * 0.35, below_horizon);

  return vec4<f32>(colour, 1.0);
}


