// Draws grass tufts as three static, world-oriented crossed quads per
// instance (18 vertices total, 60 degrees apart), plus a per-instance
// random rotation so a whole meadow does not look axis-aligned. Grass is
// never camera-facing — unlike a single billboard, three crossed blades
// read as a believable tuft silhouette from any angle, which matters more
// for something this close to the camera than it does for distant trees.
//
// No image textures are used: blade colour comes from a simple vertical
// gradient tinted per instance, and tufts fade out with distance
// (`grassViewDistanceMetres`) via alpha blending rather than a hard pop.

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
  vegetation_params: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> frame: FrameUniforms;

struct VertexIn {
  @builtin(vertex_index) vertex_index: u32,
  @location(0) local_offset: vec2<f32>,
  @location(1) uv: vec2<f32>,
  @location(2) instance_position: vec3<f32>,
  @location(3) instance_scale: f32,
  @location(4) instance_tint: f32,
};

struct VertexOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) tint: f32,
  @location(2) world_position: vec3<f32>,
  @location(3) fade: f32,
};

fn hash11(value: f32) -> f32 {
  var x = fract(value * 0.1031);
  x = x * (x + 33.33);
  return fract(x * (x + x));
}

// Shared mist term — identical in spirit to the ones in
// `clipmap_render.wgsl`/`flora_instances.wgsl`/`water.wgsl`.
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

@vertex
fn vertex_main(in: VertexIn) -> VertexOut {
  let world_up = vec3<f32>(0.0, 1.0, 0.0);
  let position_seed = in.instance_position.x * 0.053 + in.instance_position.z * 0.091;
  let base_angle = hash11(position_seed) * 1.0471976; // 0..60 degrees
  let quad_index = f32(in.vertex_index / 6u);
  let quad_angle = base_angle + quad_index * 1.0471976; // 60 degrees apart
  let right = vec3<f32>(cos(quad_angle), 0.0, sin(quad_angle));

  // Grass sways more freely than tree canopies: the whole blade bends,
  // scaled linearly with height rather than quadratically.
  let sway_phase = hash11(position_seed * 1.741) * 6.2831853;
  let sway = sin(frame.water_params.z * 2.2 + sway_phase)
    * frame.vegetation_params.x * in.uv.y * 1.1;

  let world_position = in.instance_position
    + right * (in.local_offset.x * in.instance_scale + sway)
    + world_up * in.local_offset.y * in.instance_scale;

  let view_distance = max(frame.vegetation_params.w, 1.0);
  let distance_to_camera = length(frame.camera_position.xyz - in.instance_position);
  let fade_start = view_distance * 0.8;
  let fade = 1.0 - clamp(
    (distance_to_camera - fade_start) / max(view_distance - fade_start, 1.0),
    0.0,
    1.0
  );

  var out: VertexOut;
  out.clip_position = frame.view_proj * vec4<f32>(world_position, 1.0);
  out.uv = in.uv;
  out.tint = in.instance_tint;
  out.world_position = world_position;
  out.fade = fade;
  return out;
}

@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
  if (in.fade <= 0.02) {
    discard;
  }

  // Taper the blade to a point at the tip so the quad reads as a blade
  // rather than a rectangle.
  let half_width_at_tip = 0.5 * (1.0 - in.uv.y * 0.85);
  let centred_x = in.uv.x - 0.5;

  if (abs(centred_x) > half_width_at_tip) {
    discard;
  }

  let base_colour = vec3<f32>(0.20, 0.40, 0.13);
  let tip_colour = vec3<f32>(0.42, 0.56, 0.20);
  let variation = vec3<f32>(0.06, 0.08, 0.02) * (in.tint - 0.5);
  var colour = mix(base_colour, tip_colour, in.uv.y) + variation;

  let mist = mist_factor(in.world_position, frame.camera_position.xyz);
  colour = mix(colour, frame.mist_colour.rgb, mist);

  return vec4<f32>(colour, in.fade);
}
