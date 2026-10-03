// Boulders and talus below rock outcrops: six procedural fractured blocks
// (`render/boulders.rs`), drawn instanced from the lists
// `boulder_generate.wgsl` culls into, and into the tree shadow map within
// the shadow distance. `common.wgsl` and `materials.wgsl` come first.
//
// Each boulder turns by a hash of its position and leans with the slope.
// Its stone is the rock texture projected on the world planes, with the
// terrain's macro joints and crevice shade, lichen on its upper faces,
// snow on top in snowfall and a darker, glossier skin in rain. Stones by
// the water (a stream's own, `stone_at`) are wet, darkest and glossiest
// low down, and bare of lichen.

struct BoulderIn {
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  // xyz: the base, sunk into the ground; w: metres across.
  @location(2) placed: vec4<f32>,
  // x: how far the base is sunk, yz: the slope it leans with, w: variant
  // and rank.
  @location(3) extra: vec4<f32>,
};

struct BoulderOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) world_position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  // x: metres above the ground, y: ground moisture, z: 1 on ice-arctic
  // ground, w: how close to the water (1 within half a metre, 0 from
  // 2.5 m).
  @location(2) site: vec4<f32>,
};

// A mesh vertex placed in the world: turned, scaled and leant. Returns
// the position, and the normal in `normal`.
fn place(in: BoulderIn, normal: ptr<function, vec3<f32>>) -> vec3<f32> {
  let yaw = hash12(in.placed.xz) * 6.2831853;
  let c = cos(yaw);
  let s = sin(yaw);
  let lean = in.extra.yz * 0.6;
  let local = in.position * in.placed.w;
  let turned = vec3<f32>(c * local.x - s * local.z, local.y, s * local.x + c * local.z);
  let n = vec3<f32>(c * in.normal.x - s * in.normal.z, in.normal.y, s * in.normal.x + c * in.normal.z);
  // Sheared along the slope: normals take the inverse transpose.
  *normal = normalize(vec3<f32>(n.x, n.y - dot(lean, n.xz), n.z));
  return in.placed.xyz + turned + vec3<f32>(lean.x, 0.0, lean.y) * turned.y;
}

@vertex
fn vertex_main(in: BoulderIn) -> BoulderOut {
  var normal = vec3<f32>(0.0, 1.0, 0.0);
  let position = place(in, &normal);
  let surface = surface_at(in.placed.xz);
  var out: BoulderOut;
  out.clip_position = frame.view_proj * vec4<f32>(position, 1.0);
  out.world_position = position;
  out.normal = normal;
  out.site = vec4<f32>(
    position.y - in.placed.y - in.extra.x,
    surface.g,
    select(0.0, 1.0, round(surface.a * 255.0) == 18.0),
    1.0 - smoothstep(0.5, 2.5, water_banks_at(in.placed.xz).x)
  );
  return out;
}

@fragment
fn fragment_main(in: BoulderOut) -> @location(0) vec4<f32> {
  let position = in.world_position;
  let ddx_p = dpdx(position);
  let ddy_p = dpdy(position);
  let smooth_normal = normalize(in.normal);
  // Fractured faces: half the flat facet's own normal.
  var face = normalize(cross(ddy_p, ddx_p));
  face = face * sign(dot(face, smooth_normal) + 1e-4);
  let distance = length(position - frame.camera_position.xyz);
  // The rock texture on the world planes, 4 m a repeat.
  let stone = sample_projected(MAT_ROCK, position, ddx_p, ddy_p, projection_weights(MAT_ROCK, smooth_normal), 1.0 / (4.0 * max(frame.surface.z, 0.01)), vec2<f32>(0.0));
  var albedo = stone.albedo * world.material_tints[MAT_ROCK].rgb;
  let rock = rock_detail(position, ddx_p, ddy_p, distance);
  albedo = albedo * rock.tone * (1.0 - 0.25 * rock.joint);
  let crevice = max(rock.joint, saturate((0.45 - stone.height) * 2.0));
  let lichen = rock_lichen(position, ddx_p, ddy_p, smooth_normal, in.site.y, in.site.z > 0.5);
  albedo = mix(albedo, lichen.rgb * world.material_tints[MAT_ROCK].rgb, lichen.a * (1.0 - rock.joint) * (1.0 - in.site.w));
  var normal = normalize(mix(smooth_normal, face, 0.5) + vec3<f32>(stone.detail.x, 0.0, stone.detail.y) * 0.6);
  var roughness = stone.roughness;
  // Rain darkens and glosses the stone; snowfall settles on its top.
  let ground_weather = surface_weather_at(position.xz);
  let wet = max(ground_weather.x, in.site.w * (1.0 - 0.5 * smoothstep(0.1, 0.8, in.site.x)));
  albedo = albedo * (1.0 - 0.3 * wet);
  roughness = mix(roughness, 0.2, wet * 0.8);
  let snow = ground_weather.z * smoothstep(0.55, 0.85, smooth_normal.y);
  albedo = mix(albedo, vec3<f32>(0.8, 0.84, 0.9), snow);
  normal = normalize(mix(normal, smooth_normal, snow));
  // Stone meeting the ground is in its own shade.
  let contact = mix(0.55, 1.0, smoothstep(0.0, 0.3, in.site.x));
  let ao = stone.occlusion * (1.0 - 0.45 * crevice) * contact;
  let colour = shade_surface(albedo, normal, position, ao, mix(0.35, 1.0, wet) * (1.0 - roughness), max(roughness, 0.12));
  return vec4<f32>(colour, 1.0);
}

// Shadow casters: depth only, into the tree shadow map.
@vertex
fn shadow_main(in: BoulderIn) -> @builtin(position) vec4<f32> {
  var normal = vec3<f32>(0.0, 1.0, 0.0);
  return frame.shadow_view_proj * vec4<f32>(place(in, &normal), 1.0);
}

@fragment
fn shadow_fragment() {
}
