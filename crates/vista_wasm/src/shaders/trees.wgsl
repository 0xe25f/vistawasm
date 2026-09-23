// Draws procedurally modelled trees. `common.wgsl` is prepended.
//
// Three entry-point pairs share this file:
//
// - `vertex_mesh`/`fragment_mesh`: full 3D species meshes (see
//   `render/tree_models.rs`) drawn instanced near the camera, with wind
//   sway, leaf flutter, alpha-tested foliage cards, and translucent,
//   wrap-lit foliage shading.
// - `vertex_impostor`/`fragment_impostor`: distant trees drawn as a
//   camera-facing (or crossed) quad textured with a pre-rendered image of
//   the very same mesh, so the switch is barely visible. Mesh and impostor
//   cross-fade with a shared screen-space dither, which never leaves holes.
// - `vertex_bake`/`fragment_bake`: renders each species once, at start-up,
//   into the impostor texture array.
//
// Instance data comes from the GPU culling pass (`tree_cull.wgsl`), which
// packs `species + fade * 0.5` into one float.

struct MeshIn {
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) uv: vec2<f32>,
  @location(3) params: vec4<f32>,
  @location(4) instance_position: vec3<f32>,
  @location(5) instance_scale: f32,
  @location(6) instance_rotation: f32,
  @location(7) instance_tint: f32,
  @location(8) instance_species_fade: f32,
  @location(9) instance_dryness: f32,
};

struct MeshOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) world_position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) uv: vec2<f32>,
  @location(3) @interpolate(flat) layer: i32,
  @location(4) ao: f32,
  @location(5) @interpolate(flat) species: u32,
  @location(6) @interpolate(flat) fade: f32,
  @location(7) @interpolate(flat) tint: f32,
  @location(8) @interpolate(flat) dryness: f32,
};

fn wind_direction() -> vec3<f32> {
  return normalize(vec3<f32>(0.8, 0.0, 0.6));
}

fn rotate_y(v: vec3<f32>, angle: f32) -> vec3<f32> {
  let c = cos(angle);
  let s = sin(angle);
  return vec3<f32>(v.x * c + v.z * s, v.y, -v.x * s + v.z * c);
}

fn unpack_species(value: f32) -> u32 {
  return min(u32(floor(value + 0.001)), 7u);
}

fn unpack_fade(value: f32) -> f32 {
  return saturate(fract(value + 0.0005) * 2.0);
}

// Wind: a slow whole-tree lean plus faster branch sway, both modulated by
// travelling gusts so a forest does not move in lockstep.
fn wind_offset(instance_position: vec3<f32>, weight: f32, phase: f32, height: f32) -> vec3<f32> {
  let strength = frame.vegetation.x;

  if (strength <= 0.0001 || weight <= 0.0001) {
    return vec3<f32>(0.0);
  }

  let t = time_seconds();
  let gust_phase = dot(instance_position.xz, wind_direction().xz) * 0.012 - t * 0.9;
  let gust = 0.55 + 0.45 * sin(gust_phase) * sin(gust_phase * 0.37 + 1.3);
  let lean = sin(t * 0.8 + instance_position.x * 0.05 + instance_position.z * 0.03) * 0.5 + 0.7;
  let sway = sin(t * 2.1 + phase + instance_position.x * 0.13) * 0.35;
  let amount = weight * strength * gust * (lean + sway) * clamp(height * 0.05, 0.2, 1.5);
  return wind_direction() * amount + vec3<f32>(0.0, -abs(amount) * 0.08, 0.0);
}

@vertex
fn vertex_mesh(in: MeshIn) -> MeshOut {
  let species = unpack_species(in.instance_species_fade);
  let scale = in.instance_scale;
  let tree_height = world.species[species].x * scale;
  var local = rotate_y(in.position * scale, in.instance_rotation);
  let layer = i32(in.params.x + 0.5);
  var offset = wind_offset(in.instance_position, in.params.y, in.params.w, tree_height);

  // Leaves flutter independently of the branch they hang from.
  if (layer >= 4 && frame.vegetation.x > 0.0001) {
    let t = time_seconds();
    let flutter = sin(t * 6.3 + in.params.w * 3.0 + in.position.y * 1.7) * 0.06 * in.params.y * frame.vegetation.x;
    offset = offset + rotate_y(in.normal, in.instance_rotation) * flutter * scale;
  }

  let world_position = in.instance_position + local + offset;

  var out: MeshOut;
  out.clip_position = frame.view_proj * vec4<f32>(world_position, 1.0);
  out.world_position = world_position;
  out.normal = rotate_y(in.normal, in.instance_rotation);
  out.uv = in.uv;
  out.layer = layer;
  out.ao = in.params.z;
  out.species = species;
  out.fade = unpack_fade(in.instance_species_fade);
  out.tint = in.instance_tint;
  out.dryness = in.instance_dryness;
  return out;
}

fn foliage_colour(albedo: vec3<f32>, species: u32, tint: f32, dryness: f32) -> vec3<f32> {
  var colour = albedo * world.species_tint[species].rgb * (0.82 + tint * 0.36);
  // Hot, dry climates bleach and yellow the canopy slightly.
  colour = mix(colour, colour * vec3<f32>(1.25, 1.05, 0.55), dryness * 0.35);
  return colour;
}

fn shade_foliage(
  albedo: vec3<f32>,
  normal: vec3<f32>,
  world_position: vec3<f32>,
  occlusion: f32
) -> vec3<f32> {
  let sun = sun_dir();
  let view = normalize(frame.camera_position.xyz - world_position);
  let shadow = cloud_shadow(world_position);
  // Wrap lighting: light bleeds around the rounded crown.
  let wrap = saturate((dot(normal, sun) + 0.45) / 1.45);
  // Translucency: leaves glow when back-lit by the sun.
  let back = pow(saturate(dot(-view, sun)), 3.0) * 0.6;
  let direct = sun_light() * shadow * (wrap * 0.85 + back * vec3<f32>(0.9, 1.1, 0.55));
  let ambient = sky_irradiance(normal) * occlusion;
  return albedo * (direct * occlusion + ambient * 0.75) / PI * 2.6;
}

@fragment
fn fragment_mesh(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
  let texel = textureSample(flora_texture, linear_sampler, in.uv, in.layer);
  let dither = pixel_dither(in.clip_position.xy);

  if (dither > in.fade) {
    discard;
  }

  if (in.layer >= 4) {
    if (texel.a < 0.45) {
      discard;
    }

    let albedo = foliage_colour(srgb_to_linear(texel.rgb), in.species, in.tint, in.dryness);
    let normal = normalize(in.normal);
    return vec4<f32>(shade_foliage(albedo, normal, in.world_position, in.ao), 1.0);
  }

  var normal = normalize(in.normal);

  if (!front) {
    normal = -normal;
  }

  let albedo = srgb_to_linear(texel.rgb) * (0.9 + in.tint * 0.2);
  let colour = shade_surface(albedo, normal, in.world_position, in.ao * (0.6 + texel.a * 0.4), 0.0, 0.9);
  return vec4<f32>(colour, 1.0);
}

struct ImpostorIn {
  @builtin(vertex_index) vertex_index: u32,
  @location(4) instance_position: vec3<f32>,
  @location(5) instance_scale: f32,
  @location(6) instance_rotation: f32,
  @location(7) instance_tint: f32,
  @location(8) instance_species_fade: f32,
  @location(9) instance_dryness: f32,
};

struct ImpostorOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) world_position: vec3<f32>,
  @location(1) uv: vec2<f32>,
  @location(2) normal: vec3<f32>,
  @location(3) @interpolate(flat) species: u32,
  @location(4) @interpolate(flat) fade: f32,
  @location(5) @interpolate(flat) tint: f32,
  @location(6) @interpolate(flat) dryness: f32,
};

@vertex
fn vertex_impostor(in: ImpostorIn) -> ImpostorOut {
  var corners = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(-1.0, 1.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, 1.0)
  );
  let corner = corners[in.vertex_index % 6u];
  let species = unpack_species(in.instance_species_fade);
  let bounds = world.species[species];
  let height = bounds.x * in.instance_scale;
  let radius = bounds.y * in.instance_scale;
  let up = vec3<f32>(0.0, 1.0, 0.0);
  var right: vec3<f32>;
  var facing: vec3<f32>;

  if (frame.vegetation.z > 0.5 && frame.vegetation.z < 1.5) {
    // Crossed quads: two fixed planes per tree.
    let angle = in.instance_rotation + select(0.0, 1.5707963, in.vertex_index >= 6u);
    right = vec3<f32>(cos(angle), 0.0, -sin(angle));
    facing = cross(right, up);
  } else {
    let to_camera = frame.camera_position.xyz - in.instance_position;
    let flat_length = max(length(to_camera.xz), 0.0001);
    facing = vec3<f32>(to_camera.x / flat_length, 0.0, to_camera.z / flat_length);
    right = normalize(cross(up, facing));
  }

  let sway = wind_offset(in.instance_position, corner.y * corner.y * 0.6, in.instance_rotation * 3.0, height);
  let world_position = in.instance_position + right * corner.x * radius + up * corner.y * height + sway;

  var out: ImpostorOut;
  out.clip_position = frame.view_proj * vec4<f32>(world_position, 1.0);
  out.world_position = world_position;
  out.uv = vec2<f32>(corner.x * 0.5 + 0.5, 1.0 - corner.y);
  // A rounded pseudo-normal lights the flat card like a crown.
  out.normal = normalize(right * corner.x * 0.8 + up * (corner.y - 0.35) * 0.9 + facing * 0.7);
  out.species = species;
  out.fade = unpack_fade(in.instance_species_fade);
  out.tint = in.instance_tint;
  out.dryness = in.instance_dryness;
  return out;
}

@fragment
fn fragment_impostor(in: ImpostorOut) -> @location(0) vec4<f32> {
  let texel = textureSample(impostor_texture, linear_sampler, in.uv, i32(in.species));
  let dither = pixel_dither(in.clip_position.xy);

  // Complementary to the mesh dither in the cross-fade band.
  if (texel.a < 0.5 || dither <= in.fade) {
    discard;
  }

  var albedo = srgb_to_linear(texel.rgb) * (0.82 + in.tint * 0.36);
  albedo = mix(albedo, albedo * vec3<f32>(1.25, 1.05, 0.55), in.dryness * 0.35);
  let colour = shade_foliage(albedo, normalize(in.normal), in.world_position, 0.85);
  return vec4<f32>(colour, 1.0);
}

struct BakeIn {
  @builtin(instance_index) species: u32,
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) uv: vec2<f32>,
  @location(3) params: vec4<f32>,
};

struct BakeOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) @interpolate(flat) layer: i32,
  @location(2) ao: f32,
  @location(3) normal: vec3<f32>,
  @location(4) @interpolate(flat) species: u32,
};

@vertex
fn vertex_bake(in: BakeIn) -> BakeOut {
  let bounds = world.species[in.species];
  var out: BakeOut;
  // Orthographic side view from +Z, filling the layer exactly.
  out.clip_position = vec4<f32>(
    in.position.x / bounds.y,
    in.position.y / bounds.x * 2.0 - 1.0,
    0.5 - in.position.z / (bounds.y * 4.0),
    1.0
  );
  out.uv = in.uv;
  out.layer = i32(in.params.x + 0.5);
  out.ao = in.params.z;
  out.normal = in.normal;
  out.species = in.species;
  return out;
}

@fragment
fn fragment_bake(in: BakeOut) -> @location(0) vec4<f32> {
  let texel = textureSample(flora_texture, linear_sampler, in.uv, in.layer);

  if (in.layer >= 4 && texel.a < 0.45) {
    discard;
  }

  var albedo = srgb_to_linear(texel.rgb);

  if (in.layer >= 4) {
    albedo = albedo * world.species_tint[in.species].rgb;
  }

  // Bake occlusion and a soft top-light into the impostor so it keeps the
  // mesh's depth cues when lit flat.
  let shade = in.ao * (0.78 + 0.22 * saturate(normalize(in.normal).y * 0.5 + 0.5));
  return vec4<f32>(pow(albedo * shade, vec3<f32>(1.0 / 2.2)), 1.0);
}
