// Draws grass tufts as three static, world-oriented crossed quads per
// instance (18 vertices total, 60 degrees apart), plus a per-instance
// random rotation so a whole meadow does not look axis-aligned.
// `common.wgsl` is prepended.
//
// Each quad is cut into several tapered blades in the fragment shader, so
// one tuft reads as a clump of individual blades. Colour follows the
// local climate (lush green to savannah straw), blades are alpha-tested
// and write depth so they sort correctly, and tufts thin out with a
// screen-space dither near `grassViewDistanceMetres` instead of popping.

struct VertexIn {
  @builtin(vertex_index) vertex_index: u32,
  @location(0) local_offset: vec2<f32>,
  @location(1) uv: vec2<f32>,
  @location(2) instance_position: vec3<f32>,
  @location(3) instance_scale: f32,
  @location(4) instance_tint: f32,
  @location(5) instance_dryness: f32,
};

struct VertexOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) @interpolate(flat) tint: f32,
  @location(2) world_position: vec3<f32>,
  @location(3) @interpolate(flat) fade: f32,
  @location(4) @interpolate(flat) dryness: f32,
  @location(5) normal: vec3<f32>,
};

@vertex
fn vertex_main(in: VertexIn) -> VertexOut {
  let world_up = vec3<f32>(0.0, 1.0, 0.0);
  let position_seed = in.instance_position.x * 0.053 + in.instance_position.z * 0.091;
  let base_angle = hash11(position_seed) * 1.0471976;
  let quad_index = f32(in.vertex_index / 6u);
  let quad_angle = base_angle + quad_index * 1.0471976;
  let right = vec3<f32>(cos(quad_angle), 0.0, sin(quad_angle));

  // The whole blade bends, more at the tip, with travelling gusts.
  let t = time_seconds();
  let gust = 0.6 + 0.4 * sin(dot(in.instance_position.xz, vec2<f32>(0.8, 0.6)) * 0.05 - t * 1.4);
  let sway_phase = hash11(position_seed * 1.741) * TAU;
  let sway = (sin(t * 2.4 + sway_phase) * 0.35 + 0.65) * gust * frame.vegetation.x * in.uv.y * in.uv.y * 0.9;
  let wind = normalize(vec3<f32>(0.8, 0.0, 0.6));

  let height = in.instance_scale * mix(1.0, 1.6, in.instance_dryness);
  let world_position = in.instance_position
    + right * in.local_offset.x * in.instance_scale * 1.3
    + world_up * in.local_offset.y * height
    + wind * sway * in.instance_scale;

  let view_distance = max(frame.vegetation.w, 1.0);
  let distance_to_camera = length(frame.camera_position.xyz - in.instance_position);
  let fade = 1.0 - smoothstep(view_distance * 0.7, view_distance, distance_to_camera);

  var out: VertexOut;
  out.clip_position = frame.view_proj * vec4<f32>(world_position, 1.0);
  out.uv = in.uv;
  out.tint = in.instance_tint;
  out.world_position = world_position;
  out.fade = fade;
  out.dryness = in.instance_dryness;
  // Grass normals lean towards up so tufts shade like the ground they
  // grow from rather than like vertical cards.
  out.normal = normalize(world_up * 2.0 + cross(right, world_up) * 0.5);
  return out;
}

@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
  if (in.fade <= pixel_dither(in.clip_position.xy)) {
    discard;
  }

  // Five tapered blades per quad, each with its own height and lean.
  let blades = 5.0;
  let cell = floor(in.uv.x * blades);
  let local_x = fract(in.uv.x * blades) - 0.5;
  let blade_seed = hash11(cell * 7.13 + in.tint * 31.0);
  let blade_height = 0.55 + blade_seed * 0.45;
  let lean = (blade_seed - 0.5) * 0.6 * in.uv.y;
  let v = in.uv.y / blade_height;
  let half_width = 0.42 * (1.0 - v);

  if (v > 1.0 || abs(local_x - lean) > half_width) {
    discard;
  }

  let lush_base = vec3<f32>(0.035, 0.075, 0.014);
  let lush_tip = vec3<f32>(0.16, 0.26, 0.05);
  let dry_base = vec3<f32>(0.12, 0.09, 0.035);
  let dry_tip = vec3<f32>(0.52, 0.4, 0.17);
  let base = mix(lush_base, dry_base, in.dryness);
  let tip = mix(lush_tip, dry_tip, in.dryness);
  var albedo = mix(base, tip, pow(v, 0.8)) * (0.8 + blade_seed * 0.4) * (0.85 + in.tint * 0.3);

  let sun = sun_dir();
  let view = normalize(frame.camera_position.xyz - in.world_position);
  let back = pow(saturate(dot(-view, sun)), 3.0) * 0.5 * v;
  let occlusion = 0.35 + 0.65 * v;
  let shadow = cloud_shadow(in.world_position);
  let direct = sun_light() * shadow * (saturate(dot(in.normal, sun)) * 0.8 + back);
  let colour = albedo * (direct * occlusion + sky_irradiance(in.normal) * occlusion * 0.75) / PI * 2.6;
  return vec4<f32>(colour, 1.0);
}
