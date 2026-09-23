// Tree shadow map: renders every shadow-casting tree into a light-space
// depth map. `common.wgsl` is prepended.
//
// Each tree is drawn as a single impostor quad turned to face the sun,
// alpha-tested against the impostor of its own species, so shadows have
// the real crown silhouette at a cost of two triangles per tree. On the
// tree itself, the half of the crown behind that quad falls into shadow,
// which reads as believable self-shadowing.

struct ShadowIn {
  @builtin(vertex_index) vertex_index: u32,
  @location(4) instance_position: vec3<f32>,
  @location(5) instance_scale: f32,
  @location(6) instance_rotation: f32,
  @location(7) instance_tint: f32,
  @location(8) instance_species_fade: f32,
  @location(9) instance_dryness: f32,
};

struct ShadowOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) @interpolate(flat) species: u32,
};

@vertex
fn vertex_main(in: ShadowIn) -> ShadowOut {
  var corners = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(-1.0, 1.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, 1.0)
  );
  let corner = corners[in.vertex_index % 6u];
  let species = min(u32(floor(in.instance_species_fade + 0.001)), 7u);
  let bounds = world.species[species];
  let sun = sun_dir();
  let flat_length = max(length(sun.xz), 0.0001);
  let facing = vec3<f32>(sun.x / flat_length, 0.0, sun.z / flat_length);
  let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), facing));
  let world_position = in.instance_position
    + right * corner.x * bounds.y * in.instance_scale
    + vec3<f32>(0.0, corner.y * bounds.x * in.instance_scale, 0.0);

  var out: ShadowOut;
  out.clip_position = frame.shadow_view_proj * vec4<f32>(world_position, 1.0);
  out.uv = vec2<f32>(corner.x * 0.5 + 0.5, 1.0 - corner.y);
  out.species = species;
  return out;
}

@fragment
fn fragment_main(in: ShadowOut) {
  if (textureSampleLevel(impostor_texture, linear_sampler, in.uv, i32(in.species), 1.0).a < 0.5) {
    discard;
  }
}
