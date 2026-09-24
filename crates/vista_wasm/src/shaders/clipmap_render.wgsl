// Renders terrain with procedurally generated, texture-splatted surface
// materials. `common.wgsl` is prepended to this file.
//
// Each vertex carries ten biome-driven material weights (lush grass, dry
// grass, forest floor, sand, rock, snow, mud, volcanic, ice, tundra),
// packed as twelve bytes in three u32s, plus climate data. Its normal is
// octahedron-encoded in two snorm16 values.
// The fragment shader keeps the three strongest materials, samples their
// baked albedo/height and normal/AO/roughness textures (see
// `texture_gen.wgsl`) at a near and a far scale to hide tiling, and blends
// them by height so grass grows between stones and sand settles into
// cracks rather than cross-fading like paint. Steep rock is projected
// triplanar so cliffs are not stretched. Output is linear HDR; fog and tone
// mapping happen in the composite pass (`atmosphere.wgsl`).

struct VertexIn {
  @location(0) position: vec3<f32>,
  @location(1) normal: vec2<f32>,
  @location(2) materials: vec3<u32>,
  @location(3) climate: vec4<f32>,
  @location(4) biome: vec4<u32>,
};

struct VertexOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) world_position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) materials_a: vec4<f32>,
  @location(3) materials_b: vec4<f32>,
  // xy: ice and tundra weights, z: permanent snow, w: unused.
  @location(4) materials_c: vec4<f32>,
  @location(5) climate: vec4<f32>,
  @location(6) @interpolate(flat) biome: u32,
};

// Inverse of `encode_normal` in `render/terrain_mesh.rs`.
fn decode_normal(encoded: vec2<f32>) -> vec3<f32> {
  var n = vec3<f32>(encoded.x, 1.0 - abs(encoded.x) - abs(encoded.y), encoded.y);

  if (n.y < 0.0) {
    let signs = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xz >= vec2<f32>(0.0));
    n = vec3<f32>((1.0 - abs(n.z)) * signs.x, n.y, (1.0 - abs(n.x)) * signs.y);
  }

  return normalize(n);
}

@vertex
fn vertex_main(in: VertexIn) -> VertexOut {
  var out: VertexOut;
  out.clip_position = frame.view_proj * vec4<f32>(in.position, 1.0);
  out.world_position = in.position;
  out.normal = decode_normal(in.normal);
  out.materials_a = unpack4x8unorm(in.materials.x);
  out.materials_b = unpack4x8unorm(in.materials.y);
  // Slots 10 and 11 are reserved.
  out.materials_c = vec4<f32>(unpack4x8unorm(in.materials.z).xy, f32(in.biome.w) / 255.0, 0.0);
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
const MAT_ICE: i32 = 8;
const MAT_TUNDRA: i32 = 9;
const MATERIAL_COUNT: i32 = 10;

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
    case 8: { return 14.0; }
    case 9: { return 5.0; }
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

// Average colour of each material, used when textures are switched off.
fn flat_colour(material: i32) -> vec3<f32> {
  switch material {
    case 0: { return vec3<f32>(0.05, 0.1, 0.02); }
    case 1: { return vec3<f32>(0.24, 0.18, 0.08); }
    case 2: { return vec3<f32>(0.035, 0.028, 0.016); }
    case 3: { return vec3<f32>(0.45, 0.37, 0.24); }
    case 4: { return vec3<f32>(0.14, 0.13, 0.12); }
    case 5: { return vec3<f32>(0.78, 0.82, 0.88); }
    case 6: { return vec3<f32>(0.045, 0.03, 0.02); }
    case 8: { return vec3<f32>(0.5, 0.7, 0.86); }
    case 9: { return vec3<f32>(0.1, 0.1, 0.05); }
    default: { return vec3<f32>(0.015, 0.013, 0.012); }
  }
}

fn sample_planar(
  material: i32,
  uv: vec2<f32>,
  ddx_uv: vec2<f32>,
  ddy_uv: vec2<f32>
) -> MaterialSample {
  // `surface` is uniform, so these branches never diverge.
  if (frame.surface.x < 0.5) {
    return MaterialSample(flat_colour(material), 0.5, vec2<f32>(0.0), 1.0, 0.85);
  }

  let a = textureSampleGrad(terrain_albedo, linear_sampler, uv, material, ddx_uv, ddy_uv);

  if (frame.surface.y < 0.5) {
    return MaterialSample(srgb_to_linear(a.rgb), a.a, vec2<f32>(0.0), 1.0, 0.85);
  }

  let n = textureSampleGrad(terrain_normal, linear_sampler, uv, material, ddx_uv, ddy_uv);
  return MaterialSample(srgb_to_linear(a.rgb), a.a, n.rg * 2.0 - 1.0, n.b, n.a);
}

// Projection weights for the three world planes (x: ZY, y: XZ, z: XY).
// Rock and glacier ice take triplanar weights from the geometric normal,
// as they cover steep slopes, and so does snow as the slope steepens past
// normal.y 0.9 to 0.7, with no seam where the projection changes. The rest
// project from above. Near-zero weights are dropped, so flat ground takes
// one sample, not three.
fn projection_weights(material: i32, normal: vec3<f32>) -> vec3<f32> {
  var steep = select(0.0, 1.0, material == MAT_ROCK || material == MAT_ICE);

  if (material == MAT_SNOW) {
    steep = 1.0 - smoothstep(0.7, 0.9, normal.y);
  }

  let sharp = pow(abs(normal), vec3<f32>(4.0));
  let triplanar = sharp / max(sharp.x + sharp.y + sharp.z, 0.0001);
  let weights = max(mix(vec3<f32>(0.0, 1.0, 0.0), triplanar, steep) - 0.02, vec3<f32>(0.0));
  return weights / max(weights.x + weights.y + weights.z, 0.0001);
}

// One material sample at a scale (`inv` is 1 / metres per tile), on the
// planes of `weights`. Texture gradients are explicit, so skipping planes
// is safe in any control flow.
fn sample_projected(
  material: i32,
  position: vec3<f32>,
  ddx_p: vec3<f32>,
  ddy_p: vec3<f32>,
  weights: vec3<f32>,
  inv: f32,
  offset: vec2<f32>
) -> MaterialSample {
  var result = MaterialSample(vec3<f32>(0.0), 0.0, vec2<f32>(0.0), 0.0, 0.0);

  for (var axis = 0; axis < 3; axis = axis + 1) {
    let w = weights[axis];

    if (w > 0.0) {
      var uv = position.xz;
      var du = ddx_p.xz;
      var dv = ddy_p.xz;

      if (axis == 0) {
        uv = position.zy;
        du = ddx_p.zy;
        dv = ddy_p.zy;
      } else if (axis == 2) {
        uv = position.xy;
        du = ddx_p.xy;
        dv = ddy_p.xy;
      }

      let s = sample_planar(material, uv * inv + offset, du * inv, dv * inv);
      result.albedo = result.albedo + s.albedo * w;
      result.height = result.height + s.height * w;
      result.detail = result.detail + s.detail * w;
      result.occlusion = result.occlusion + s.occlusion * w;
      result.roughness = result.roughness + s.roughness * w;
    }
  }

  return result;
}

// A single far-scale sample: the low-detail path for distant terrain.
fn sample_material_far(
  material: i32,
  position: vec3<f32>,
  ddx_p: vec3<f32>,
  ddy_p: vec3<f32>,
  normal: vec3<f32>
) -> MaterialSample {
  let far_inv = 1.0 / (material_scale(material) * max(frame.surface.z, 0.01) * 5.3);
  return sample_projected(material, position, ddx_p, ddy_p, projection_weights(material, normal), far_inv, vec2<f32>(0.37, 0.61));
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
  let inv = 1.0 / (material_scale(material) * max(frame.surface.z, 0.01));
  let weights = projection_weights(material, normal);
  var near = sample_projected(material, position, ddx_p, ddy_p, weights, inv, vec2<f32>(0.0));

  if (far_blend > 0.01) {
    let far = sample_projected(material, position, ddx_p, ddy_p, weights, inv / 5.3, vec2<f32>(0.37, 0.61));
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
    case 15u: { return vec3<f32>(0.6, 0.5, 0.62); }
    case 16u: { return vec3<f32>(0.62, 0.78, 0.95); }
    case 17u: { return vec3<f32>(0.97, 0.99, 1.0); }
    case 18u: { return vec3<f32>(0.75, 0.92, 1.0); }
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
    case 8: { return vec3<f32>(0.45, 0.75, 1.0); }
    case 9: { return vec3<f32>(0.55, 0.55, 0.3); }
    default: { return vec3<f32>(0.6, 0.1, 0.05); }
  }
}

// Crevasses open across the direction the ice flows, where it speeds up
// over steeper ground (8 to 30 degrees). Ice flows downslope, so a crack
// across the flow follows a contour line: cracks are drawn every few
// metres of height, which spaces them closer on steeper ice. Noise bends
// them into arcs and breaks them into separate crevasses. Working from
// height and world position, rather than rotating into a per-pixel flow
// frame, keeps the pattern stable. Derivatives come from the caller, so
// this is safe inside a per-pixel branch.
fn crevasses(
  position: vec3<f32>,
  ddx_p: vec3<f32>,
  ddy_p: vec3<f32>,
  normal: vec3<f32>,
  distance: f32
) -> f32 {
  let slope = acos(clamp(normal.y, -1.0, 1.0)) * 57.29578;
  let steepness = smoothstep(8.0, 11.0, slope) * (1.0 - smoothstep(27.0, 30.0, slope));
  let fade = 1.0 - smoothstep(250.0, 1000.0, distance);

  if (steepness * fade <= 0.001) {
    return 0.0;
  }

  let scale = 1.0 / 110.0;
  let noise = textureSampleGrad(noise_texture, linear_sampler, position.xz * scale, ddx_p.xz * scale, ddy_p.xz * scale);
  // One crack every `rise` metres of height, bowed by the noise.
  let rise = 4.0;
  let phase = (position.y + (noise.g - 0.5) * 9.0) / rise;
  let crack_index = floor(phase);
  let offset = abs(fract(phase) - 0.5);
  // Each crack breaks into separate crevasses that taper at their ends.
  let run = noise.r * 5.0 + hash12(vec2<f32>(crack_index, 7.7)) * 3.0;
  let segment = step(0.4, hash12(vec2<f32>(crack_index, floor(run))));
  let width = 0.08 * sin(fract(run) * PI);
  // Thin cracks shrink below a pixel in the distance; fade them rather than
  // let them shimmer.
  let footprint = max(abs(ddx_p.y), abs(ddy_p.y)) / rise;
  let crack = (1.0 - smoothstep(width * 0.5, width + footprint, offset)) * saturate(width / max(footprint, 0.0001));
  // Crevasse fields come and go across the glacier.
  let field = smoothstep(0.35, 0.6, noise.a);
  return crack * segment * field * steepness * fade;
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

  // Past the render distance the composite covers this pixel with fog, so
  // skip the shading.
  if (beyond_render_distance(distance)) {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
  }

  // Sea floor so deep that the water above it is fully opaque (see
  // `opacity` in `water.wgsl`: from 1.14 x clarity), even in a wave
  // trough: most of the skirt beyond the map, and the open sea. The water
  // pass draws over it.
  if (frame.water_origin.z > 0.5
    && frame.water_shallow.w - position.y > max(frame.water_params.z, 0.1) * 1.2 + frame.wave_params.x) {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
  }

  // Past the detail distance each material takes one far-scale sample
  // instead of up to eight. Textures there are so minified that the two
  // look the same.
  let low_detail = distance > frame.distances.y;

  var weights = array<f32, 10>(
    in.materials_a.x, in.materials_a.y, in.materials_a.z, in.materials_a.w,
    in.materials_b.x, in.materials_b.y, in.materials_b.z, in.materials_b.w,
    in.materials_c.x, in.materials_c.y
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
      for (var i = 1; i < MATERIAL_COUNT; i = i + 1) {
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

  for (var i = 0; i < MATERIAL_COUNT; i = i + 1) {
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
      if (low_detail) {
        samples[k] = sample_material_far(top[k], position, ddx_p, ddy_p, geometric_normal);
      } else {
        samples[k] = sample_material(top[k], position, ddx_p, ddy_p, geometric_normal, far_blend);
      }

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
  var ice_amount = 0.0;
  var volcanic_amount = 0.0;
  var volcanic_height = 1.0;
  var wet_amount = 0.0;

  for (var k = 0; k < 3; k = k + 1) {
    if (top_weight[k] > 0.02) {
      let w = max(blend_height[k] - max_height + 0.22, 0.0);
      albedo = albedo + samples[k].albedo * world.material_tints[top[k]].rgb * w;
      detail = detail + samples[k].detail * w;
      occlusion = occlusion + samples[k].occlusion * w;
      roughness = roughness + samples[k].roughness * w;
      total = total + w;

      if (top[k] == MAT_SNOW) {
        snow_amount = snow_amount + w;
      }

      if (top[k] == MAT_ICE) {
        ice_amount = ice_amount + w;
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
  ice_amount = ice_amount / total;
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

  var crevasse = 0.0;

  if (ice_amount > 0.01) {
    crevasse = crevasses(position, ddx_p, ddy_p, geometric_normal, distance) * ice_amount;
    // Light scattered inside glacier ice comes out blue where the sun does
    // not reach directly, and deepest in the cracks.
    let shaded = (1.0 - saturate(dot(geometric_normal, sun_dir()))) * 0.35;
    albedo = mix(albedo, srgb_to_linear(vec3<f32>(0.25, 0.55, 0.85)), ice_amount * shaded);
    albedo = mix(albedo, srgb_to_linear(vec3<f32>(0.04, 0.16, 0.34)), crevasse);
    roughness = mix(roughness, 0.3, crevasse);
  }

  // Damp ground near water, in wet biomes, and after rain darkens and
  // turns glossy; flat hollows collect puddles that mirror the sky.
  let shore = saturate(1.0 - (position.y - frame.water_shallow.w) / 2.5);
  let rain_wet = frame.weather.z * (1.0 - snow_amount);
  let wetness = saturate(max(max(wet_amount * 0.7, shore * 0.8), rain_wet * 0.85) + (moisture - 0.8) * 0.5);
  albedo = albedo * (1.0 - wetness * 0.35);
  roughness = mix(roughness, 0.18, wetness * 0.8);
  var blend_height_avg = 0.0;

  for (var k = 0; k < 3; k = k + 1) {
    if (top_weight[k] > 0.02) {
      blend_height_avg = max(blend_height_avg, samples[k].height);
    }
  }

  let puddle = rain_wet * smoothstep(0.93, 0.99, geometric_normal.y)
    * (1.0 - smoothstep(0.25, 0.42, blend_height_avg)) * smoothstep(0.35, 0.9, frame.weather.z);

  // Detail normal in world space (the textures are projected on XZ).
  let detail_strength = mix(1.0, 0.35, far_blend) * (1.0 - puddle);
  var normal = normalize(geometric_normal + vec3<f32>(detail.x, 0.0, detail.y) * detail_strength);

  // Settled snow from the weather covers upward-facing ground, and so does
  // snow that never melts: old snow patches in the hollows of the tundra.
  // Glacier ice shows its own snow and ice through its material weights.
  let permanent = in.materials_c.z * (1.0 - ice_amount);
  var lying = 0.0;

  if (permanent > 0.001) {
    // Two scales of noise, so the patches never visibly repeat.
    let broad_scale = 1.0 / 1730.0;
    let broad = textureSampleGrad(noise_texture, linear_sampler, position.xz * broad_scale + vec2<f32>(0.23, 0.61), ddx_p.xz * broad_scale, ddy_p.xz * broad_scale).r;
    let field = macro_noise.r * 0.45 + broad * 0.35 + (1.0 - blend_height_avg) * 0.2;
    lying = smoothstep(1.0 - permanent, 1.2 - permanent, field) * permanent;
  }

  let settled = max(frame.weather.w, lying) * smoothstep(0.55, 0.85, geometric_normal.y);

  if (settled > 0.001) {
    let snow_cover = saturate(settled * (0.75 + blend_height_avg * 0.5));
    albedo = mix(albedo, vec3<f32>(0.8, 0.84, 0.9), snow_cover);
    roughness = mix(roughness, 0.45, snow_cover);
    snow_amount = max(snow_amount, snow_cover);
  }
  // Snow settles on the upward-facing side of bumps.
  normal = normalize(mix(normal, geometric_normal, snow_amount * 0.5));

  let ao = occlusion * cavity * (1.0 - crevasse * 0.6);
  let specular = mix(0.35, 1.0, wetness) * (1.0 - roughness);
  var colour = shade_surface(albedo, normal, position, ao, specular + snow_amount * 0.25, max(roughness, 0.12));

  if (puddle > 0.01) {
    let view = normalize(frame.camera_position.xyz - position);
    let fresnel = 0.02 + 0.98 * pow(1.0 - saturate(view.y), 5.0);
    let reflection = sky_radiance(reflect(-view, vec3<f32>(0.0, 1.0, 0.0)));
    colour = mix(colour, colour * 0.4 + reflection * fresnel, puddle);
  }

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
