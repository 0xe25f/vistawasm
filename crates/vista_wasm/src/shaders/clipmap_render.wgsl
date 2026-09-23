// Renders terrain with procedurally generated, texture-splatted surface
// materials. `common.wgsl` is prepended to this file.
//
// Each vertex carries eight biome-driven material weights (lush grass, dry
// grass, forest floor, sand, rock, snow, mud, volcanic) plus climate data.
// The fragment shader keeps the three strongest materials, samples their
// baked albedo/height and normal/AO/roughness textures (see
// `texture_gen.wgsl`) at a near and a far scale to hide tiling, and blends
// them by height so grass grows between stones and sand settles into
// cracks rather than cross-fading like paint. Steep rock is projected
// triplanar so cliffs are not stretched. Output is linear HDR; fog and tone
// mapping happen in the composite pass (`atmosphere.wgsl`).

struct VertexIn {
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) materials_a: vec4<f32>,
  @location(3) materials_b: vec4<f32>,
  @location(4) climate: vec4<f32>,
  @location(5) biome: vec4<u32>,
};

struct VertexOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) world_position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) materials_a: vec4<f32>,
  @location(3) materials_b: vec4<f32>,
  @location(4) climate: vec4<f32>,
  @location(5) @interpolate(flat) biome: u32,
};

@vertex
fn vertex_main(in: VertexIn) -> VertexOut {
  var out: VertexOut;
  out.clip_position = frame.view_proj * vec4<f32>(in.position, 1.0);
  out.world_position = in.position;
  out.normal = in.normal;
  out.materials_a = in.materials_a;
  out.materials_b = in.materials_b;
  out.climate = in.climate;
  out.biome = in.biome.x;
  return out;
}

const MAT_LUSH: i32 = 0;
const MAT_DRY: i32 = 1;
const MAT_FOREST: i32 = 2;
const MAT_SAND: i32 = 3;
const MAT_ROCK: i32 = 4;
const MAT_SNOW: i32 = 5;
const MAT_MUD: i32 = 6;
const MAT_VOLCANIC: i32 = 7;

// Metres covered by one repeat of each material texture (near scale).
fn material_scale(material: i32) -> f32 {
  switch material {
    case 0: { return 4.0; }
    case 1: { return 4.5; }
    case 2: { return 5.0; }
    case 3: { return 6.5; }
    case 4: { return 12.0; }
    case 5: { return 9.0; }
    case 6: { return 5.5; }
    default: { return 10.0; }
  }
}

struct MaterialSample {
  albedo: vec3<f32>,
  height: f32,
  detail: vec2<f32>,
  occlusion: f32,
  roughness: f32,
};

fn sample_planar(
  material: i32,
  uv: vec2<f32>,
  ddx_uv: vec2<f32>,
  ddy_uv: vec2<f32>
) -> MaterialSample {
  let a = textureSampleGrad(terrain_albedo, linear_sampler, uv, material, ddx_uv, ddy_uv);
  let n = textureSampleGrad(terrain_normal, linear_sampler, uv, material, ddx_uv, ddy_uv);
  return MaterialSample(srgb_to_linear(a.rgb), a.a, n.rg * 2.0 - 1.0, n.b, n.a);
}

// Two scales, cross-faded with distance, so close-up detail never tiles
// visibly and distant ground does not shimmer.
fn sample_material(
  material: i32,
  position: vec3<f32>,
  ddx_p: vec3<f32>,
  ddy_p: vec3<f32>,
  normal: vec3<f32>,
  far_blend: f32
) -> MaterialSample {
  let scale = material_scale(material);

  if (material == MAT_ROCK) {
    // Triplanar projection weighted by the geometric normal.
    var weights = pow(abs(normal), vec3<f32>(4.0));
    weights = weights / max(weights.x + weights.y + weights.z, 0.0001);
    let inv = 1.0 / scale;
    let x = sample_planar(material, position.zy * inv, ddx_p.zy * inv, ddy_p.zy * inv);
    let y = sample_planar(material, position.xz * inv, ddx_p.xz * inv, ddy_p.xz * inv);
    let z = sample_planar(material, position.xy * inv, ddx_p.xy * inv, ddy_p.xy * inv);
    var result: MaterialSample;
    result.albedo = x.albedo * weights.x + y.albedo * weights.y + z.albedo * weights.z;
    result.height = x.height * weights.x + y.height * weights.y + z.height * weights.z;
    result.detail = x.detail * (weights.x + weights.z) + y.detail * weights.y;
    result.occlusion = x.occlusion * weights.x + y.occlusion * weights.y + z.occlusion * weights.z;
    result.roughness = x.roughness * weights.x + y.roughness * weights.y + z.roughness * weights.z;

    if (far_blend > 0.01) {
      let far_inv = inv / 5.7;
      let far = sample_planar(material, position.xz * far_inv + vec2<f32>(0.31, 0.73), ddx_p.xz * far_inv, ddy_p.xz * far_inv);
      result.albedo = mix(result.albedo, far.albedo, far_blend * 0.6);
    }

    return result;
  }

  let inv = 1.0 / scale;
  var near = sample_planar(material, position.xz * inv, ddx_p.xz * inv, ddy_p.xz * inv);

  if (far_blend > 0.01) {
    let far_inv = inv / 5.3;
    let far = sample_planar(
      material,
      position.xz * far_inv + vec2<f32>(0.37, 0.61),
      ddx_p.xz * far_inv,
      ddy_p.xz * far_inv
    );
    near.albedo = mix(near.albedo, far.albedo, far_blend * 0.65);
    near.detail = mix(near.detail, far.detail, far_blend * 0.5);
    near.height = mix(near.height, far.height, far_blend * 0.5);
  }

  return near;
}

fn biome_debug_colour(biome: u32) -> vec3<f32> {
  switch biome {
    case 0u: { return vec3<f32>(0.55, 0.8, 0.3); }
    case 1u: { return vec3<f32>(0.4, 0.62, 0.22); }
    case 2u: { return vec3<f32>(0.2, 0.5, 0.18); }
    case 3u: { return vec3<f32>(0.08, 0.32, 0.1); }
    case 4u: { return vec3<f32>(0.6, 0.55, 0.4); }
    case 5u: { return vec3<f32>(0.55, 0.55, 0.58); }
    case 6u: { return vec3<f32>(0.35, 0.2, 0.18); }
    case 7u: { return vec3<f32>(0.9, 0.25, 0.05); }
    case 8u: { return vec3<f32>(0.85, 0.72, 0.35); }
    case 9u: { return vec3<f32>(0.95, 0.88, 0.62); }
    case 10u: { return vec3<f32>(0.5, 0.45, 0.42); }
    case 11u: { return vec3<f32>(0.15, 0.7, 0.35); }
    case 12u: { return vec3<f32>(0.02, 0.45, 0.2); }
    case 13u: { return vec3<f32>(0.3, 0.38, 0.25); }
    default: { return vec3<f32>(0.1, 0.25, 0.55); }
  }
}

fn material_debug_colour(material: i32) -> vec3<f32> {
  switch material {
    case 0: { return vec3<f32>(0.2, 0.7, 0.2); }
    case 1: { return vec3<f32>(0.8, 0.7, 0.3); }
    case 2: { return vec3<f32>(0.35, 0.22, 0.1); }
    case 3: { return vec3<f32>(0.95, 0.85, 0.55); }
    case 4: { return vec3<f32>(0.5, 0.5, 0.5); }
    case 5: { return vec3<f32>(1.0, 1.0, 1.0); }
    case 6: { return vec3<f32>(0.3, 0.2, 0.15); }
    default: { return vec3<f32>(0.6, 0.1, 0.05); }
  }
}

@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
  let position = in.world_position;
  let geometric_normal = normalize(in.normal);
  // Derivatives are taken once, in uniform control flow, then passed to
  // `textureSampleGrad` so material sampling can branch freely.
  let ddx_p = dpdx(position);
  let ddy_p = dpdy(position);
  let distance = length(position - frame.camera_position.xyz);

  var weights = array<f32, 8>(
    in.materials_a.x, in.materials_a.y, in.materials_a.z, in.materials_a.w,
    in.materials_b.x, in.materials_b.y, in.materials_b.z, in.materials_b.w
  );

  let debug_view = i32(frame.sky_tint.w + 0.5);

  if (debug_view > 0) {
    var debug_colour = vec3<f32>(0.0);

    if (debug_view == 1) {
      let t = saturate((position.y - frame.water_shallow.w) / 1800.0);
      debug_colour = mix(vec3<f32>(0.1, 0.35, 0.1), vec3<f32>(0.95, 0.9, 0.85), t);
    } else if (debug_view == 2) {
      let slope = 1.0 - geometric_normal.y;
      debug_colour = mix(vec3<f32>(0.1, 0.6, 0.1), vec3<f32>(0.9, 0.1, 0.1), saturate(slope * 3.0));
    } else if (debug_view == 3) {
      debug_colour = geometric_normal * 0.5 + 0.5;
    } else if (debug_view == 4) {
      var best = 0;
      for (var i = 1; i < 8; i = i + 1) {
        if (weights[i] > weights[best]) {
          best = i;
        }
      }
      debug_colour = material_debug_colour(best);
    } else {
      debug_colour = biome_debug_colour(in.biome);
    }

    let light = 0.35 + 0.65 * saturate(dot(geometric_normal, sun_dir()));
    return vec4<f32>(srgb_to_linear(debug_colour) * light * 1.6, 1.0);
  }

  // Keep the three strongest materials.
  var top = array<i32, 3>(0, 0, 0);
  var top_weight = array<f32, 3>(-1.0, -1.0, -1.0);

  for (var i = 0; i < 8; i = i + 1) {
    let w = weights[i];

    if (w > top_weight[0]) {
      top_weight[2] = top_weight[1];
      top[2] = top[1];
      top_weight[1] = top_weight[0];
      top[1] = top[0];
      top_weight[0] = w;
      top[0] = i;
    } else if (w > top_weight[1]) {
      top_weight[2] = top_weight[1];
      top[2] = top[1];
      top_weight[1] = w;
      top[1] = i;
    } else if (w > top_weight[2]) {
      top_weight[2] = w;
      top[2] = i;
    }
  }

  let far_blend = smoothstep(35.0, 260.0, distance);
  var samples: array<MaterialSample, 3>;
  var blend_height = array<f32, 3>(0.0, 0.0, 0.0);
  var max_height = -10.0;

  for (var k = 0; k < 3; k = k + 1) {
    if (top_weight[k] > 0.02) {
      samples[k] = sample_material(top[k], position, ddx_p, ddy_p, geometric_normal, far_blend);
      // Height-based blend: materials whose texture sits higher win the
      // transition, producing natural, crisp borders.
      blend_height[k] = top_weight[k] + samples[k].height * 0.45;
      max_height = max(max_height, blend_height[k]);
    }
  }

  var albedo = vec3<f32>(0.0);
  var detail = vec2<f32>(0.0);
  var occlusion = 0.0;
  var roughness = 0.0;
  var total = 0.0;
  var snow_amount = 0.0;
  var volcanic_amount = 0.0;
  var volcanic_height = 1.0;
  var wet_amount = 0.0;

  for (var k = 0; k < 3; k = k + 1) {
    if (top_weight[k] > 0.02) {
      let w = max(blend_height[k] - max_height + 0.22, 0.0);
      albedo = albedo + samples[k].albedo * w;
      detail = detail + samples[k].detail * w;
      occlusion = occlusion + samples[k].occlusion * w;
      roughness = roughness + samples[k].roughness * w;
      total = total + w;

      if (top[k] == MAT_SNOW) {
        snow_amount = snow_amount + w;
      }

      if (top[k] == MAT_VOLCANIC) {
        volcanic_amount = volcanic_amount + w;
        volcanic_height = samples[k].height;
      }

      if (top[k] == MAT_MUD) {
        wet_amount = wet_amount + w;
      }
    }
  }

  total = max(total, 0.0001);
  albedo = albedo / total;
  detail = detail / total;
  occlusion = occlusion / total;
  roughness = roughness / total;
  snow_amount = snow_amount / total;
  volcanic_amount = volcanic_amount / total;
  wet_amount = wet_amount / total;

  // Climate tinting: grass yellows where it is hot and dry and deepens
  // where it is wet, so neighbouring biomes shade into each other.
  let moisture = in.climate.x;
  let temperature = in.climate.y;
  let heat = in.climate.z;
  let cavity = in.climate.w;
  let grass_share = saturate((weights[0] + weights[1]) * 1.2);
  let dry_tint = vec3<f32>(1.12, 0.98, 0.72);
  let wet_tint = vec3<f32>(0.82, 1.0, 0.86);
  let climate_tint = mix(dry_tint, wet_tint, saturate(moisture * 1.3 - 0.15));
  albedo = albedo * mix(vec3<f32>(1.0), climate_tint, grass_share * 0.6);

  // Large-scale variation breaks up any remaining uniformity.
  let macro_uv = position.xz / 380.0;
  let macro_noise = textureSampleGrad(noise_texture, linear_sampler, macro_uv, ddx_p.xz / 380.0, ddy_p.xz / 380.0);
  albedo = albedo * (0.84 + macro_noise.g * 0.32) * mix(vec3<f32>(1.0), vec3<f32>(1.05, 1.0, 0.9), macro_noise.b * 0.5);

  // Damp ground near water and in wet biomes darkens and turns glossy.
  let shore = saturate(1.0 - (position.y - frame.water_shallow.w) / 2.5);
  let wetness = saturate(max(wet_amount * 0.7, shore * 0.8) + (moisture - 0.8) * 0.5);
  albedo = albedo * (1.0 - wetness * 0.35);
  roughness = mix(roughness, 0.18, wetness * 0.8);

  // Detail normal in world space (the textures are projected on XZ).
  let detail_strength = mix(1.0, 0.35, far_blend);
  var normal = normalize(geometric_normal + vec3<f32>(detail.x, 0.0, detail.y) * detail_strength);
  // Snow settles on the upward-facing side of bumps.
  normal = normalize(mix(normal, geometric_normal, snow_amount * 0.5));

  let ao = occlusion * cavity;
  let specular = mix(0.35, 1.0, wetness) * (1.0 - roughness);
  var colour = shade_surface(albedo, normal, position, ao, specular + snow_amount * 0.25, max(roughness, 0.12));

  // Snow glitter.
  if (snow_amount > 0.05) {
    let sparkle = pow(hash12(floor(position.xz * 38.0)), 180.0);
    colour = colour + sun_light() * sparkle * snow_amount * saturate(dot(normal, sun_dir())) * cloud_shadow(position) * 2.0;
  }

  // Lava glowing in the cracks of caldera basalt.
  if (heat > 0.01 && volcanic_amount > 0.01) {
    let cracks = pow(saturate(1.0 - volcanic_height * 1.35), 3.0);
    let pulse = 0.75 + 0.25 * sin(time_seconds() * 0.9 + position.x * 0.05 + position.z * 0.03);
    colour = colour + vec3<f32>(4.5, 1.2, 0.18) * cracks * heat * volcanic_amount * pulse;
  }

  return vec4<f32>(colour, 1.0);
}
