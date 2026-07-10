// Renders a flat water plane with an animated procedural ripple, a
// fresnel-driven blend towards the sky colour, and a specular sun highlight.
//
// The plane geometry is a single quad sized to the terrain footprint; all
// visual detail comes from this shader, so no wave texture is required.

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
};

@group(0) @binding(0)
var<uniform> frame: FrameUniforms;

struct VertexIn {
  @location(0) position: vec3<f32>,
};

struct VertexOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) world_position: vec3<f32>,
};

@vertex
fn vertex_main(in: VertexIn) -> VertexOut {
  var out: VertexOut;
  out.clip_position = frame.view_proj * vec4<f32>(in.position, 1.0);
  out.world_position = in.position;
  return out;
}

// Shared mist term — identical in spirit to `clipmap_render.wgsl`'s. Water
// sits at or near `baseHeightMetres`/the sea level, so `riseAboveWater`
// mist should visibly touch it.
fn mist_factor(world_position: vec3<f32>, camera_position: vec3<f32>) -> f32 {
  let density = frame.mist_params.x;

  if (density <= 0.0001) {
    return 0.0;
  }

  let base_height = frame.mist_params.y;
  let falloff = max(frame.mist_params.z, 0.001);
  let noise_strength = frame.mist_params.w;
  let sea_level = frame.mist_colour.w;

  let height_term = clamp((base_height + falloff - world_position.y) / falloff, 0.0, 1.0);
  let water_term = clamp(1.0 - abs(world_position.y - sea_level) / max(falloff * 0.5, 1.0), 0.0, 1.0);
  var strength = max(height_term, water_term * 0.6);

  if (noise_strength > 0.0001) {
    let drift = frame.water_params.z * 0.15;
    let n = 0.5 + 0.5 * sin(world_position.x * 0.01 + drift) * cos(world_position.z * 0.013 - drift * 0.7);
    strength = mix(strength, strength * n, noise_strength);
  }

  let distance = length(world_position - camera_position);
  let distance_fade = clamp(1.0 - distance / 6000.0, 0.15, 1.0);

  return clamp(strength * density * distance_fade, 0.0, 1.0);
}

@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
  let wave_scale = max(frame.water_params.x, 0.0);
  let reflectivity = clamp(frame.water_params.y, 0.0, 1.0);
  let time = frame.water_params.z;

  let ripple = sin(in.world_position.x * 0.05 + time * 1.3) * 0.5
    + sin(in.world_position.z * 0.07 - time * 0.9) * 0.5;
  let normal = normalize(vec3<f32>(ripple * wave_scale * 0.08, 1.0, ripple * wave_scale * 0.05));

  let view_direction = normalize(frame.camera_position.xyz - in.world_position);
  let fresnel = pow(1.0 - clamp(dot(normal, view_direction), 0.0, 1.0), 3.0);

  let deep_colour = vec3<f32>(0.04, 0.16, 0.28);
  let shallow_colour = vec3<f32>(0.14, 0.42, 0.48);
  let sky_colour = vec3<f32>(0.55, 0.70, 0.85);

  let sun_direction = normalize(frame.sun_direction.xyz);
  let highlight = pow(
    clamp(dot(reflect(-sun_direction, normal), view_direction), 0.0, 1.0),
    40.0
  );

  var colour = mix(deep_colour, shallow_colour, clamp(0.35 + ripple * 0.15, 0.0, 1.0));
  colour = mix(colour, sky_colour, fresnel * (0.35 + reflectivity * 0.5));
  colour += vec3<f32>(1.0, 0.98, 0.9) * highlight * frame.sun_colour_intensity.w;

  let mist = mist_factor(in.world_position, frame.camera_position.xyz);
  colour = mix(colour, frame.mist_colour.rgb, mist);

  let alpha = clamp(0.55 + fresnel * 0.35, 0.0, 0.95);

  return vec4<f32>(colour, alpha);
}


