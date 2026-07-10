// Draws procedural tree billboards from per-instance transforms.
//
// Each billboard rotates around the vertical axis only (a "cylindrical"
// billboard), so trees stay upright while still facing the camera
// horizontally in the `Billboard` tree style. The `CrossQuad`/`Mesh`
// styles instead draw two STATIC, world-oriented quads 90 degrees apart
// per tree (never camera-facing), which reads as real volume/parallax
// from any angle instead of a single flat cutout — the vertex shader
// picks each quad's fixed orientation from `@builtin(vertex_index)` (0..6
// is the first quad, 6..12 the second) plus a per-instance random
// rotation, so a whole forest of crossed quads is not axis-aligned
// identically. See `frame.vegetation_params.z` and
// `docs/environment-upgrade-plan.md` §2.
//
// The trunk and canopy shapes are carved out procedurally in the fragment
// shader with `discard`, so no image textures are required. Canopy
// silhouettes are perturbed per instance by `speciesVariation` so
// neighbouring trees do not look identical, and the canopy sways in the
// wind proportional to `windStrength`.

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
  @location(3) shape_seed: f32,
};

// A cheap deterministic hash used only for visual variety (canopy shape,
// per-tree cross-quad rotation, wind phase) — not for placement, so it
// need not match the CPU-side seeded RNG. It is a pure function of each
// tree's already-deterministic world position, so the resulting visuals
// stay reproducible for a given terrain seed.
fn hash11(value: f32) -> f32 {
  var x = fract(value * 0.1031);
  x = x * (x + 33.33);
  return fract(x * (x + x));
}

// Shared mist term, identical in spirit to the one in `clipmap_render.wgsl`
// and `water.wgsl`, so a tree standing in a misty valley is tinted the
// same as the ground around it. See `MistOptions` in `vista_types` and
// `docs/environment-upgrade-plan.md` §5.
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
  let cross_quad = frame.vegetation_params.z > 0.5;
  let position_seed = in.instance_position.x * 0.037 + in.instance_position.z * 0.071;

  var right: vec3<f32>;

  if (cross_quad) {
    // Two static, world-oriented planes 90 degrees apart, with a random
    // per-instance rotation so a forest of crossed quads is not
    // axis-aligned identically.
    let base_angle = hash11(position_seed) * 1.5707963;
    let quad_angle = select(base_angle, base_angle + 1.5707963, in.vertex_index >= 6u);
    right = vec3<f32>(cos(quad_angle), 0.0, sin(quad_angle));
  } else {
    let to_camera = frame.camera_position.xyz - in.instance_position;
    let flat_length = max(length(vec2<f32>(to_camera.x, to_camera.z)), 0.0001);
    let flat = vec3<f32>(to_camera.x / flat_length, 0.0, to_camera.z / flat_length);
    right = normalize(cross(world_up, flat));
  }

  // Wind sway: only the canopy (uv.y above the trunk) bends, proportional
  // to the configured strength; the trunk stays rigid. Zero
  // `windStrength` reproduces today's static rendering exactly.
  let sway_phase = hash11(position_seed * 1.913) * 6.2831853;
  let sway = sin(frame.water_params.z * 1.6 + sway_phase)
    * frame.vegetation_params.x * in.uv.y * in.uv.y * 2.2;

  let world_position = in.instance_position
    + right * (in.local_offset.x * in.instance_scale + sway)
    + world_up * in.local_offset.y * in.instance_scale;

  var out: VertexOut;
  out.clip_position = frame.view_proj * vec4<f32>(world_position, 1.0);
  out.uv = in.uv;
  out.tint = in.instance_tint;
  out.world_position = world_position;
  out.shape_seed = hash11(position_seed * 2.917);
  return out;
}

@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
  let trunk_colour = vec3<f32>(0.32, 0.24, 0.16);
  let canopy_base = vec3<f32>(0.16, 0.36, 0.14);
  let canopy_variation = vec3<f32>(0.10, 0.14, 0.04) * in.tint;
  let centred_x = in.uv.x - 0.5;
  let variation_strength = frame.vegetation_params.y;

  // Per-tree canopy shape jitter: widens/narrows and raises/lowers the
  // canopy ellipse so neighbouring trees do not look identical. Zero
  // `speciesVariation` reproduces the original single fixed silhouette
  // exactly (today's default look).
  let width_jitter = 1.0 + (in.shape_seed - 0.5) * 0.5 * variation_strength;
  let height_jitter = (in.shape_seed - 0.5) * 0.16 * variation_strength;

  if (in.uv.y < 0.32) {
    // Trunk: a thin vertical strip below the canopy.
    if (abs(centred_x) > 0.06) {
      discard;
    }

    return vec4<f32>(trunk_colour, 1.0);
  }

  // Canopy: an elliptical mask so the flat quad reads as foliage.
  let canopy_centre_y = 0.68 + height_jitter;
  let canopy_y = (in.uv.y - canopy_centre_y) / 0.32;
  let ellipse = (centred_x * centred_x) / (0.5 * 0.5 * width_jitter * width_jitter) + canopy_y * canopy_y;

  if (ellipse > 1.0) {
    discard;
  }

  let shade = clamp(1.0 - ellipse * 0.4, 0.0, 1.0);
  var colour = (canopy_base + canopy_variation) * shade;

  let mist = mist_factor(in.world_position, frame.camera_position.xyz);
  colour = mix(colour, frame.mist_colour.rgb, mist);

  return vec4<f32>(colour, 1.0);
}


