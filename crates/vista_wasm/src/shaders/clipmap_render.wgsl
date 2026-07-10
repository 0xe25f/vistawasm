// Renders terrain as a lit triangle mesh with height-based material blending,
// a simple distance haze approximation, and an optional height-based ground
// mist term (see `MistOptions` in `vista_types` and
// `docs/environment-upgrade-plan.md` §5). Haze is a uniform, distance-only
// blend towards sky colour; mist is a separate, height-based ground fog
// that pools in valleys and near water — the two are independently
// controlled and compose together below.
//
// This is an interim single-resolution renderer: the whole terrain mesh is
// baked on the CPU (position, normal, and material weights) and drawn as one
// draw call. Clipmap LOD ring paging remains future work.

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
  @location(1) normal: vec3<f32>,
  @location(2) material: vec4<f32>,
};

struct VertexOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) world_position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) material: vec4<f32>,
};

@vertex
fn vertex_main(in: VertexIn) -> VertexOut {
  var out: VertexOut;
  out.clip_position = frame.view_proj * vec4<f32>(in.position, 1.0);
  out.world_position = in.position;
  out.normal = in.normal;
  out.material = in.material;
  return out;
}

// Height-based ground mist. `density <= 0` (the `Off` style default) skips
// all of this cheaply. `Flat` uses just the height/water falloff terms;
// `Volumetric` additionally multiplies in a drifting noise term so mist
// thickness varies across the terrain rather than being a uniform layer.
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
  let grass_colour = vec3<f32>(0.30, 0.45, 0.22);
  let rock_colour = vec3<f32>(0.42, 0.38, 0.34);
  let snow_colour = vec3<f32>(0.92, 0.94, 0.97);
  let wet_mud_colour = vec3<f32>(0.24, 0.20, 0.15);
  let sky_colour = vec3<f32>(0.55, 0.70, 0.85);

  let normal = normalize(in.normal);
  let sun_direction = normalize(frame.sun_direction.xyz);
  let n_dot_l = max(dot(normal, sun_direction), 0.0);
  let ambient = 0.25;
  let light = ambient + n_dot_l * frame.sun_colour_intensity.w;

  let total_weight = max(
    in.material.x + in.material.y + in.material.z + in.material.w,
    0.0001
  );
  let albedo = (
    grass_colour * in.material.x +
    rock_colour * in.material.y +
    snow_colour * in.material.z +
    wet_mud_colour * in.material.w
  ) / total_weight;

  let lit_colour = albedo * frame.sun_colour_intensity.rgb * light;

  // Blend towards the sky colour with distance to approximate atmospheric
  // haze without a full scattering simulation.
  let haze_distance = max(frame.fog.x, 1.0);
  let distance = length(in.world_position - frame.camera_position.xyz);
  let haze = clamp(distance / haze_distance, 0.0, 1.0);
  let exposure = frame.fog.y;
  var final_colour = mix(lit_colour * exposure, sky_colour, haze * haze);

  // Ground mist is applied on top of the haze blend, so it can only add
  // ground-level fog and never removes the existing distance haze.
  let mist = mist_factor(in.world_position, frame.camera_position.xyz);
  final_colour = mix(final_colour, frame.mist_colour.rgb, mist);

  return vec4<f32>(final_colour, 1.0);
}

