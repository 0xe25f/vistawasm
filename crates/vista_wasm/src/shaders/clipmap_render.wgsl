// Renders terrain with procedurally generated, texture-splatted surface
// materials. `common.wgsl` and `materials.wgsl` (material sampling and
// bare rock) are prepended to this file.
//
// Each vertex carries twelve biome-driven material weights (lush grass,
// dry grass, forest floor, sand, rock, snow, mud, volcanic, ice, tundra,
// gravel, scree), packed as twelve bytes in three u32s, plus climate data.
// Its normal is octahedron-encoded in two snorm16 values.
// The fragment shader keeps the three strongest materials, samples their
// baked albedo/height and normal/AO/roughness textures (see
// `texture_gen.wgsl`) at a near and a far scale to hide tiling, and blends
// them by height so grass grows between stones and sand settles into
// cracks rather than cross-fading like paint. Steep rock is projected
// triplanar so cliffs are not stretched. Output is linear HDR; fog and tone
// mapping happen in the composite pass (`atmosphere.wgsl`).

// The grass density mask (`GroundData::grass_mask`) at ground layer
// resolution, capped to density 4; 1 x 1 and neutral without one. The
// terrain alone binds it, as the composite and cloud passes have no
// sampled texture to spare, and its grass sheen follows the tufts by it.
@group(3) @binding(0) var grass_mask_texture: texture_2d<f32>;

// The grass density mask's multiplier at a world position, 0 to 2.
fn grass_mask_at(xz: vec2<f32>) -> f32 {
  let size = max(world.terrain2.xy, vec2<f32>(1.0));
  let uv = ((xz + world.terrain.xy) / world.terrain.zw + 0.5) / size;
  return density_multiplier(textureSampleLevel(grass_mask_texture, clamp_sampler, uv, 0.0).r);
}

// The channel distance field (`render/channel_field.rs`): an atlas 64
// tiles of 16 x 16 texels a row, r the signed distance to the drawn
// water's edge and g the owning channel's flow byte
// (`channel_field::encode_flow`); and
// its slot words: field texels per metre (f32 bits), map tiles across and
// down, 1 when there are tiles, then each map tile's atlas slot plus 1
// (0 without one).
@group(3) @binding(1) var channel_atlas: texture_2d<f32>;
@group(3) @binding(2) var<storage, read> channel_slots: array<u32>;

// `FIELD_RANGE_METRES`, `SPEED_STEP` and `BAND_STEP` in
// `render/channel_field.rs`.
const FIELD_RANGE: f32 = 16.0;
const FIELD_SPEED_STEP: f32 = 0.5;
const FIELD_BAND_STEP: f32 = 0.07;

// A field texel's atlas slot plus 1, or 0 where no tile holds it.
fn channel_slot(t: vec2<i32>) -> u32 {
  let columns = i32(channel_slots[1]);

  if (any(t < vec2<i32>(0)) || t.x / 16 >= columns || t.y / 16 >= i32(channel_slots[2])) {
    return 0u;
  }

  return channel_slots[4u + u32((t.y / 16) * columns + t.x / 16)];
}

// One field texel: its distance in metres and its flow byte, or the
// field's range and -1 where no tile holds it.
fn channel_texel(t: vec2<i32>) -> vec2<f32> {
  let slot = channel_slot(t);

  if (slot == 0u) {
    return vec2<f32>(FIELD_RANGE, -1.0);
  }

  let s = slot - 1u;
  let at = vec2<i32>(i32(s % 64u) * 16 + t.x % 16, i32(s / 64u) * 16 + t.y % 16);
  let texel = textureLoad(channel_atlas, at, 0).rg;
  return vec2<f32>((texel.r * 255.0 - 127.5) / 127.5 * FIELD_RANGE, round(texel.g * 255.0));
}

struct ChannelEdge {
  // Metres to the drawn water's edge, negative under the water.
  distance: f32,
  // The owning channel's speed (metres per second), the band its bed
  // covers beside the water (metres), and how much its banks are rock
  // walls (0 to 1), read between texels.
  speed: f32,
  band: f32,
  rock: f32,
  // How much the stream has bank meshes, which draw its margin and face
  // (0 to 1).
  meshed: f32,
  // The unit direction to the water, across the field's slope.
  toward: vec2<f32>,
  // 1 where the field holds this point, 0 where the wet-bank field and
  // the samples' own beds must serve.
  held: f32,
};

// A field texel's speed, band, rock walls and bank meshes.
fn channel_flow(texel: vec2<f32>) -> vec4<f32> {
  let flow = u32(max(texel.y, 0.0));
  let band = f32(flow & 15u);
  return vec4<f32>(f32(flow >> 6u) * FIELD_SPEED_STEP, FIELD_BAND_STEP * band * band, f32((flow >> 4u) & 1u), f32((flow >> 5u) & 1u));
}

// The channel field at a world position, read between texels; held
// where the nearest texel has a tile: `ChannelField::at`. Texels with
// no tile take the nearest's speed and band, so a bed's edge never
// steps at a tile's.
fn channel_field_at(xz: vec2<f32>) -> ChannelEdge {
  var edge = ChannelEdge(FIELD_RANGE, 0.0, 0.0, 0.0, 0.0, vec2<f32>(0.0), 0.0);

  if (channel_slots[3] == 0u || !over_terrain(xz)) {
    return edge;
  }

  let f = (xz + world.terrain.xy) * bitcast<f32>(channel_slots[0]);
  let base = floor(f);
  let w = f - base;
  let b = vec2<i32>(base);

  // Most of the terrain is far from water: one slot read says so.
  if (channel_slot(b) == 0u && channel_slot(b + 1) == 0u) {
    return edge;
  }

  let t00 = channel_texel(b);
  let t10 = channel_texel(b + vec2<i32>(1, 0));
  let t01 = channel_texel(b + vec2<i32>(0, 1));
  let t11 = channel_texel(b + vec2<i32>(1, 1));
  let near = select(select(t00, t10, w.x >= 0.5), select(t01, t11, w.x >= 0.5), w.y >= 0.5);

  if (near.y < 0.0) {
    return edge;
  }

  let own = channel_flow(near);
  let f00 = select(own, channel_flow(t00), t00.y >= 0.0);
  let f10 = select(own, channel_flow(t10), t10.y >= 0.0);
  let f01 = select(own, channel_flow(t01), t01.y >= 0.0);
  let f11 = select(own, channel_flow(t11), t11.y >= 0.0);
  let flow = mix(mix(f00, f10, w.x), mix(f01, f11, w.x), w.y);
  edge.distance = mix(mix(t00.x, t10.x, w.x), mix(t01.x, t11.x, w.x), w.y);
  edge.speed = flow.x;
  edge.band = flow.y;
  edge.rock = flow.z;
  edge.meshed = flow.w;
  let slope = vec2<f32>(mix(t10.x - t00.x, t11.x - t01.x, w.y), mix(t01.x - t00.x, t11.x - t10.x, w.x));
  edge.toward = -slope / max(length(slope), 1.0e-4);
  edge.held = 1.0;
  return edge;
}

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
  // xy: ice and tundra weights, z: permanent snow, w: gravel weight.
  @location(4) materials_c: vec4<f32>,
  @location(5) climate: vec4<f32>,
  @location(6) @interpolate(flat) biome: u32,
  // The share of the ground under tree crowns (`canopy_at`).
  @location(7) canopy: f32,
  @location(8) scree: f32,
  // What the samples stamped (`SurfaceSample::bed`): the loose bed's
  // share and the rock wall's.
  @location(9) bed: vec2<f32>,
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
  let slots_c = unpack4x8unorm(in.materials.z);
  out.materials_c = vec4<f32>(slots_c.xy, f32(in.biome.w) / 255.0, slots_c.z);
  out.scree = slots_c.w;
  out.climate = in.climate;
  out.biome = in.biome.x;
  out.bed = vec2<f32>(f32(in.biome.y >> 4u), f32(in.biome.y & 15u)) / 15.0;
  // Leaf litter lies only under trees: forest floor is at most the share
  // of the ground under crowns. What moisture would make forest floor
  // where no trees stand is lush grass on gentle ground, and rougher
  // grass, as under shrubs, on steep ground.
  let canopy = canopy_at(in.position.xz);
  let litter = out.materials_a.z;
  let spare = max(litter - canopy, 0.0);
  let gentle = smoothstep(0.75, 0.9, out.normal.y);
  out.materials_a.x = out.materials_a.x + spare * (0.6 + 0.4 * gentle);
  out.materials_a.y = out.materials_a.y + spare * 0.4 * (1.0 - gentle);
  out.materials_a.z = litter - spare;
  // Under dense canopy the floor is leaf litter, over grass and rock.
  let shade = smoothstep(0.5, 0.9, canopy);
  let rock = out.materials_b.x * shade * 0.5;
  out.materials_a.z = out.materials_a.z + (out.materials_a.x + out.materials_a.y) * shade + rock;
  out.materials_a.x = out.materials_a.x * (1.0 - shade);
  out.materials_a.y = out.materials_a.y * (1.0 - shade);
  out.materials_b.x = out.materials_b.x - rock;
  out.canopy = canopy;
  return out;
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

  // Close up the near scale of rock and scree is magnified and soft: a
  // third scale, 3.3 times finer, keeps the stone crisp within about
  // 40 m, shading it by its own relief rather than recolouring it.
  if ((material == MAT_ROCK || material == MAT_SCREE) && far_blend < 0.05) {
    let fine = sample_projected(material, position, ddx_p, ddy_p, weights, inv * 3.3, vec2<f32>(0.71, 0.23));
    let close = 1.0 - far_blend * 20.0;
    near.albedo = near.albedo * mix(1.0, 0.75 + 0.5 * fine.height, 0.6 * close);
    near.detail = near.detail + fine.detail * 0.6 * close;
    near.height = mix(near.height, fine.height, 0.25 * close);
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
    case 10: { return vec3<f32>(0.45, 0.4, 0.55); }
    case 11: { return vec3<f32>(0.72, 0.6, 0.42); }
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

  var weights = array<f32, 12>(
    in.materials_a.x, in.materials_a.y, in.materials_a.z, in.materials_a.w,
    in.materials_b.x, in.materials_b.y, in.materials_b.z, in.materials_b.w,
    in.materials_c.x, in.materials_c.y, in.materials_c.w, in.scree
  );

  // Near the camera, river beds follow the drawn channels: the field's
  // loose bed (gravel at 1 m/s and over, sand from 0.4, mud below, sand
  // at mouths near sea level) and rock walls replace what the samples
  // stamped, which is squares 12 to 30 m across. Further off, and where
  // the field's band is wider than it holds, the samples' beds stay.
  var channel = ChannelEdge(FIELD_RANGE, 0.0, 0.0, 0.0, 0.0, vec2<f32>(0.0), 0.0);

  if (!low_detail) {
    channel = channel_field_at(position.xz);
    // Until a pixel spans half a field texel: past that the samples'
    // beds read the same.
    let footprint = max(length(ddx_p.xz), length(ddy_p.xz)) * bitcast<f32>(channel_slots[0]);
    channel.held = channel.held * (1.0 - smoothstep(0.25, 0.5, footprint));
    // Streams with bank meshes have their margins drawn by them.
    let field_bed = channel.held * (1.0 - step(FIELD_RANGE * 0.9, channel.band)) * (1.0 - channel.meshed);

    if (field_bed > 0.001) {
      let mouth = 1.0 - smoothstep(0.0, 1.5, position.y - world.terrain2.w);
      let gravel = smoothstep(0.9, 1.1, channel.speed) * (1.0 - mouth);
      let mud = (1.0 - smoothstep(0.3, 0.5, channel.speed)) * (1.0 - mouth) * (1.0 - gravel);
      // Lakes and pools have no band: their beds stay the ground's.
      let cover = (1.0 - smoothstep(0.6 * channel.band, channel.band, channel.distance)) * step(0.01, channel.band);
      let rock = cover * channel.rock;
      let loose = cover - rock;
      // Take away what the samples stamped, in proportion to what each
      // stamped material holds, and give back the share the stamp took
      // from the rest.
      let three = weights[MAT_GRAVEL] + weights[MAT_SAND] + weights[MAT_MUD];
      let stamped = vec2<f32>(min(in.bed.x, three), min(in.bed.y, weights[MAT_ROCK]));
      let left = (three - stamped.x) / max(three, 0.0001);
      let restore = 1.0 / max(1.0 - stamped.x - stamped.y, 0.2);
      var field_weights = weights;
      var natural = 0.0;

      for (var i = 0; i < MATERIAL_COUNT; i = i + 1) {
        var w = weights[i];

        if (i == MAT_GRAVEL || i == MAT_SAND || i == MAT_MUD) {
          w = w * left;
        }

        if (i == MAT_ROCK) {
          w = w - stamped.y;
        }

        field_weights[i] = w * restore;
        natural = natural + field_weights[i];
      }

      // A sample the stamp covered whole kept none of its ground: grass,
      // lush where the climate is moist, fills in beside the bed.
      let bare = max(1.0 - natural, 0.0);
      field_weights[MAT_LUSH] = field_weights[MAT_LUSH] + bare * saturate(in.climate.x * 1.5);
      field_weights[MAT_DRY] = field_weights[MAT_DRY] + bare * (1.0 - saturate(in.climate.x * 1.5));

      for (var i = 0; i < MATERIAL_COUNT; i = i + 1) {
        field_weights[i] = field_weights[i] * (1.0 - cover);
      }

      field_weights[MAT_GRAVEL] = field_weights[MAT_GRAVEL] + loose * gravel;
      field_weights[MAT_MUD] = field_weights[MAT_MUD] + loose * mud;
      field_weights[MAT_SAND] = field_weights[MAT_SAND] + loose * (1.0 - gravel - mud);
      field_weights[MAT_ROCK] = field_weights[MAT_ROCK] + rock;

      for (var i = 0; i < MATERIAL_COUNT; i = i + 1) {
        weights[i] = mix(weights[i], field_weights[i], field_bed);
      }
    }
  }

  // Outcrop edges between vertices are straight, a triangle's edge: a
  // noise a few metres across lobes and frays them, so turf and scree
  // meet the rock in bays and tongues.
  let stone_share = weights[MAT_ROCK];

  if (stone_share > 0.02 && stone_share < 0.98 && !low_detail) {
    let lobe_scale = 1.0 / 7.0;
    let lobe = textureSampleGrad(noise_texture, linear_sampler, position.xz * lobe_scale, ddx_p.xz * lobe_scale, ddy_p.xz * lobe_scale).a;
    weights[MAT_ROCK] = saturate(stone_share + (lobe - 0.5) * 0.9 * min(stone_share, 1.0 - stone_share));
  }

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
  // Within 120 m rock and scree read as solid: parallax at most 8 cm
  // deep, fading out from 80 m, and skipped where they are under 0.3 of
  // the ground, so blends do not pay for it.
  var textured = position;
  let stony = weights[MAT_ROCK] + weights[MAT_SCREE];

  if (!low_detail && distance < 120.0 && stony > 0.3 && frame.surface.x > 0.5) {
    let stone = select(MAT_ROCK, MAT_SCREE, weights[MAT_SCREE] > weights[MAT_ROCK]);
    textured = parallax(position, ddx_p, ddy_p, geometric_normal, stone, 0.08 * (1.0 - smoothstep(80.0, 120.0, distance)));
  }

  var samples: array<MaterialSample, 3>;
  var blend_height = array<f32, 3>(0.0, 0.0, 0.0);
  var max_height = -10.0;

  for (var k = 0; k < 3; k = k + 1) {
    if (top_weight[k] > 0.02) {
      if (low_detail) {
        samples[k] = sample_material_far(top[k], position, ddx_p, ddy_p, geometric_normal);
      } else {
        samples[k] = sample_material(top[k], textured, ddx_p, ddy_p, geometric_normal, far_blend);
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
  var gravel_amount = 0.0;
  var rock_amount = 0.0;
  var rock_height = 1.0;
  var scree_amount = 0.0;
  var scree_height = 1.0;

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

      if (top[k] == MAT_GRAVEL) {
        gravel_amount = gravel_amount + w;
      }

      if (top[k] == MAT_ROCK) {
        rock_amount = rock_amount + w;
        rock_height = samples[k].height;
      }

      if (top[k] == MAT_SCREE) {
        scree_amount = scree_amount + w;
        scree_height = samples[k].height;
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
  gravel_amount = gravel_amount / total;
  rock_amount = rock_amount / total;
  scree_amount = scree_amount / total;

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
  // Distance to water and bankside greening.
  var banks = vec2<f32>(40.0, 0.0);

  if (frame.rivers.w > 0.5 && over_terrain(position.xz)) {
    banks = water_banks_at(position.xz);
  }

  // Turf by rivers and lakes is fresher: greener and more saturated, not
  // darker.
  let fresh = banks.y * grass_share;
  albedo = albedo * mix(vec3<f32>(1.0), vec3<f32>(0.94, 1.08, 0.86), fresh);
  let luma = dot(albedo, vec3<f32>(0.2126, 0.7152, 0.0722));
  albedo = max(mix(vec3<f32>(luma), albedo, 1.0 + 0.3 * fresh), vec3<f32>(0.0));

  // Riparian scrub too far to draw: where its instances thin out with
  // distance, as the tree cull thins them, a darker, more saturated green
  // lines the water in their place, so a stream still reads as a line of
  // green. None where no scrub grows (`vegetation::SCRUB_SPECIES`).
  if (frame.waterside.x > 0.0 && in.biome < 15u && in.biome != 7u && in.biome != 14u) {
    let near = frame.waterside.x / max(distance, 1.0);
    let kept = clamp(near * near, frame.waterside.y, 1.0)
      * (1.0 - smoothstep(0.8 * frame.waterside.z, frame.waterside.z, distance));
    let line = smoothstep(0.5, 1.5, banks.x) * (1.0 - smoothstep(4.0, 7.0, banks.x));
    let scrub = (1.0 - kept) * line * saturate(banks.y * 1.5) * 0.55;
    albedo = albedo * mix(vec3<f32>(1.0), vec3<f32>(0.62, 0.78, 0.52), scrub);
  }

  // Large-scale variation breaks up any remaining uniformity.
  let macro_uv = position.xz / 380.0;
  let macro_noise = textureSampleGrad(noise_texture, linear_sampler, macro_uv, ddx_p.xz / 380.0, ddy_p.xz / 380.0);
  albedo = albedo * (0.84 + macro_noise.g * 0.32) * mix(vec3<f32>(1.0), vec3<f32>(1.05, 1.0, 0.9), macro_noise.b * 0.5);

  // Bare rock (`materials.wgsl`): jointed blocks and ledges that keep a cliff's
  // structure at every distance, lichen, and water streaks down steep
  // faces. Cracks and the gaps between scree stones fall into shade:
  // `crevice` darkens only the sky and bounce light, up to 45 %.
  var crevice = 0.0;
  var tilt = vec3<f32>(0.0);

  if (rock_amount > 0.02 && frame.surface.x > 0.5) {
    let rock = rock_detail(position, ddx_p, ddy_p, distance);
    albedo = albedo * mix(1.0, rock.tone * (1.0 - 0.25 * rock.joint), rock_amount);
    crevice = max(rock.joint, saturate((0.45 - rock_height) * 2.0)) * rock_amount;
    tilt = rock.tilt * rock_amount;
    let lichen = rock_lichen(position, ddx_p, ddy_p, geometric_normal, moisture, in.biome == 18u);
    albedo = mix(albedo, lichen.rgb * world.material_tints[MAT_ROCK].rgb, lichen.a * rock_amount * (1.0 - rock.joint));
    albedo = albedo * (1.0 - rock_streaks(position, ddx_p, ddy_p, geometric_normal, moisture) * rock_amount);
  }

  if (scree_amount > 0.02) {
    crevice = max(crevice, saturate((0.35 - scree_height) * 2.5) * scree_amount);
  }

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
  // The weather's wetness, puddles and snow where they fell (or the
  // weather's single values without the surface weather map).
  let ground_weather = surface_weather_at(position.xz);
  let rain_wet = ground_weather.x * (1.0 - snow_amount);
  // Bankside greening is not soaked ground, so it does not count here.
  var wetness = saturate(max(max(wet_amount * 0.7, shore * 0.8), rain_wet * 0.85)
    + (moisture - 0.45 * banks.y - 0.8) * 0.5);
  albedo = albedo * (1.0 - wetness * 0.35);
  roughness = mix(roughness, 0.18, wetness * 0.8);
  // Wet banks: a thin margin within 2 m of a river, lake or waterfall is
  // darker and glossy, and muddy where it is gentle.
  if (frame.rivers.w > 0.5 && over_terrain(position.xz)) {
    // The drawn banks where the channel field holds them.
    let water_distance = mix(banks.x, channel.distance, channel.held);
    let bank = 1.0 - smoothstep(0.0, 2.0, water_distance);

    if (bank > 0.001) {
      let gentle = smoothstep(0.88, 0.97, geometric_normal.y) * (1.0 - snow_amount);
      albedo = mix(albedo * (1.0 - 0.3 * bank), WET_SOIL, bank * gentle * 0.35);
      roughness = mix(roughness, 0.35, bank);
      wetness = max(wetness, bank * 0.6);
      // Wet stones by the water are darker and glossier than wet ground.
      let stones = gravel_amount * (1.0 - smoothstep(0.0, 2.0, water_distance));
      albedo = albedo * (1.0 - 0.3 * stones);
      roughness = mix(roughness, 0.12, stones);
      wetness = max(wetness, stones);
    }
  }

  var blend_height_avg = 0.0;

  for (var k = 0; k < 3; k = k + 1) {
    if (top_weight[k] > 0.02) {
      blend_height_avg = max(blend_height_avg, samples[k].height);
    }
  }

  var puddle = 0.0;

  if (ground_weather.w > 0.5) {
    // The map's puddle water fills the low parts of the ground's own
    // texture first, and spreads over more of it as it deepens.
    let water = saturate(ground_weather.y * 2.5);
    puddle = water * smoothstep(0.93, 0.99, geometric_normal.y) * (1.0 - snow_amount)
      * (1.0 - smoothstep(0.2 + 0.25 * water, 0.35 + 0.35 * water, blend_height_avg));
  } else {
    puddle = rain_wet * smoothstep(0.93, 0.99, geometric_normal.y)
      * (1.0 - smoothstep(0.25, 0.42, blend_height_avg)) * smoothstep(0.35, 0.9, frame.weather.z);
  }

  // Detail normal in world space (the textures are projected on XZ).
  let detail_strength = mix(1.0, 0.35, far_blend) * (1.0 - puddle);
  var normal = normalize(geometric_normal + vec3<f32>(detail.x, 0.0, detail.y) * detail_strength + tilt);

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

  let settled = max(ground_weather.z, lying) * smoothstep(0.55, 0.85, geometric_normal.y);

  if (settled > 0.001) {
    let snow_cover = saturate(settled * (0.75 + blend_height_avg * 0.5));
    albedo = mix(albedo, vec3<f32>(0.8, 0.84, 0.9), snow_cover);
    roughness = mix(roughness, 0.45, snow_cover);
    snow_amount = max(snow_amount, snow_cover);
  }
  // Snow settles on the upward-facing side of bumps.
  normal = normalize(mix(normal, geometric_normal, snow_amount * 0.5));

  // Beside rivers too wide for bank meshes, a steep bank is cut: over its
  // first metre from the water it is a face of wet earth and roots,
  // turned to the water, under the dark line of the turf lip's shadow
  // (the bank meshes' profile, `BankShape`, as shading). Rock walls stay
  // rock.
  if (channel.held > 0.001 && channel.distance > -0.3 && channel.distance < 2.0) {
    let steep = (1.0 - smoothstep(0.7, 0.9, geometric_normal.y)) * channel.held * (1.0 - channel.rock)
      * (1.0 - channel.meshed) * (1.0 - snow_amount);
    let face = steep * smoothstep(-0.3, 0.05, channel.distance) * (1.0 - smoothstep(0.8, 1.2, channel.distance));
    let lip = steep * smoothstep(1.1, 1.25, channel.distance) * (1.0 - smoothstep(1.3, 1.6, channel.distance));
    let roots_scale = vec2<f32>(5.0, 0.8);
    let roots = textureSampleGrad(noise_texture, linear_sampler, position.xz * roots_scale.x + vec2<f32>(0.0, position.y * roots_scale.y), ddx_p.xz * roots_scale.x, ddy_p.xz * roots_scale.x);
    let earth = mix(WET_SOIL * (0.8 + 0.4 * roots.b), vec3<f32>(0.28, 0.21, 0.13), smoothstep(0.6, 0.7, roots.r) * 0.6);
    albedo = mix(albedo, earth, face * 0.85) * (1.0 - 0.55 * lip);
    normal = normalize(normal + vec3<f32>(channel.toward.x, 0.0, channel.toward.y) * 1.5 * face);
    wetness = max(wetness, face * 0.5);
  }

  // Damp shade under canopy: mossier litter in wet climates, a little
  // more saturated and glossier ground, and beyond the tree shadows'
  // reach, a coarse dapple of the crowns overhead.
  let canopy = in.canopy;

  if (canopy > 0.01) {
    let litter = weights[MAT_FOREST] / max(weights[MAT_LUSH] + weights[MAT_DRY] + weights[MAT_FOREST] + weights[MAT_ROCK], 0.001);
    albedo = mix(albedo, albedo * vec3<f32>(0.75, 1.05, 0.62), canopy * litter * saturate((moisture - 0.5) * 2.5));
    let grey = dot(albedo, vec3<f32>(0.2126, 0.7152, 0.0722));
    albedo = max(mix(vec3<f32>(grey), albedo, 1.0 + 0.05 * canopy), vec3<f32>(0.0));
    roughness = roughness * mix(1.0, 0.85, canopy);

    if (distance > frame.vegetation3.z) {
      albedo = albedo * (1.0 - 0.5 * canopy * (0.4 + 0.6 * canopy_clumps_at(position.xz, distance)));
    }
  }

  let ao = occlusion * cavity * (1.0 - crevasse * 0.6) * (1.0 - 0.45 * crevice);
  let specular = mix(0.35, 1.0, wetness) * (1.0 - roughness);
  // Grass the tufts hand over to (`grass_generate.wgsl`): as a meadow's
  // tufts thin out with distance, the ground takes on their look by
  // exactly the share they give up (`grass::sheen_share`), so tufts and
  // ground together look like the full meadow at every distance and from
  // any height, with no ring where the tufts end. How much ground the
  // tufts hide depends on the view: from above only their cover, at a
  // slant their sides as well (`grass::apparent_cover`), so the sheen
  // takes a larger share sooner seen from above. The ground takes the
  // tufts' mean colour and light, the glow of blades lit from behind
  // when looking into the sun, and a fine fuzz of blade shadows. From
  // twice the view distance it fades back to the ground's own grass,
  // which already reads as a meadow from that far. The fuzz is sampled
  // with the pixel's own gradients, so it averages away rather than
  // shimmers as it shrinks below a pixel. The grass density mask scales
  // it as it scales the tufts.
  let view_distance = max(frame.vegetation.w, 1.0);
  let radius = frame.vegetation2.z;
  let start = handover_start(radius, view_distance);
  var handover = 0.0;

  // Nearer than the handover's start (and than where thinned tufts stop
  // growing taller), the tufts are the whole look; beyond four times the
  // view distance the sheen has faded out. Only the ground between pays.
  if (frame.vegetation2.y > 0.0 && grass_share > 0.001 && distance > min(start, 1.8 * radius)
    && distance < 4.0 * view_distance) {
    let meadow = min(frame.vegetation2.y * grass_share * grass_mask_at(position.xz), 1.0)
      * (1.0 - snow_amount) * (1.0 - puddle);

    if (meadow > 0.001) {
      let to_eye = normalize(frame.camera_position.xyz - position);
      let sine = dot(to_eye, geometric_normal);
      // Thinned tufts past their growth limit show less side for the
      // ground they cover (`grass::thinned_side`), so the sheen makes that
      // up too.
      let thinned = apparent_cover(meadow, sine, min(1.8 * radius / max(distance, 1.0), 1.0));
      let full = apparent_cover(meadow, sine, 1.0);
      handover = sheen_share(full, thinned, tuft_share(distance, start, view_distance))
        * (1.0 - smoothstep(2.0 * view_distance, 4.0 * view_distance, distance));
      let dry = weights[MAT_DRY] / max(weights[MAT_LUSH] + weights[MAT_DRY], 0.001);
      // The blades' glow, from their tips most, over their direct light.
      let sun = sun_dir();
      let glow = pow(saturate(dot(-to_eye, sun)), 3.0) * 0.29 / max(sun.y, 0.2);
      albedo = mix(albedo, meadow_albedo(dry, meadow_depth(min(meadow, 0.9), sine, 1.0)) * (1.0 + min(glow, 1.5)), handover);
    }
  }

  // Tufts are lit as if level, whatever the ground's bumps, and are matte.
  var lit_normal = normal;

  if (handover > 0.0) {
    lit_normal = normalize(mix(normal, vec3<f32>(0.0, 1.0, 0.0), handover));
  }

  var colour = shade_surface(albedo, lit_normal, position, ao, specular * (1.0 - handover) + snow_amount * 0.25, max(roughness, 0.12));

  if (handover > 0.001) {
    let fuzz = textureSampleGrad(noise_texture, linear_sampler, position.xz * 3.1, ddx_p.xz * 3.1, ddy_p.xz * 3.1).r;
    colour = colour * mix(1.0, 0.88 + 0.24 * fuzz, handover);
  }

  if (puddle > 0.01) {
    let view = normalize(frame.camera_position.xyz - position);
    let fresnel = 0.02 + 0.98 * pow(1.0 - saturate(view.y), 5.0);
    // Mirror-flat, but for the rings raindrops make while it rains.
    var surface_normal = vec3<f32>(0.0, 1.0, 0.0);

    if (frame.weather.x > 0.01 && distance < 60.0) {
      let rings = rain_rings(position.xz) * 0.18 * saturate(frame.weather.x * 2.0);
      surface_normal = normalize(vec3<f32>(rings.x, 1.0, rings.y));
    }

    let reflection = sky_radiance(reflect(-view, surface_normal));
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

// --- Canopy layer ---------------------------------------------------------
//
// Beyond the canopy distance individual trees give way to a shell over
// the far bands of the terrain mesh, at every other vertex
// (`terrain_mesh::canopy_indices`): each vertex is raised by the canopy
// height, `mix(8, 28 m, species height)`, plus the ground's rise above
// its edges (`terrain_mesh::canopy_lift`), easing in over the thinnest
// tenth of cover at forest edges, so the shell stands clear of the ground
// and never fights it for depth. Crowns cover the share of it a forest
// hides from this view; the rest is discarded, showing the ground between
// them. Seen at a slant the trees' sides hide ground too, so the share is
// `1 - exp(-crowns x (1 + side / top x tan(view angle)))`, with crowns
// the crown area per unit of ground and each crown's side 0.11 of its
// height times its top. Between 0.8 and 1 times the canopy distance the
// trees drop out by hash in the cull pass as the shell fades in crown by
// crown, so there is never a band with neither. Off the map there is no
// cover: skirt vertices are pulled onto the footprint's edge, which
// collapses their triangles.

struct CanopyOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) world_position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  // Crown area per unit of ground (`canopy_at` is 1 - exp(-crowns)).
  @location(2) crowns: f32,
  @location(3) @interpolate(flat) species: u32,
  @location(4) climate: vec2<f32>,
};

// How tall each species' canopy stands, from 0 (8 m) to 1 (28 m), in
// `TreeSpecies` order.
fn species_height(species: u32) -> f32 {
  switch species {
    case 0u: { return 0.7; }
    case 1u: { return 0.8; }
    case 2u: { return 0.75; }
    case 3u: { return 0.5; }
    case 4u: { return 1.0; }
    case 5u: { return 0.6; }
    case 6u: { return 0.3; }
    default: { return 0.0; }
  }
}

@vertex
fn canopy_vertex_main(in: VertexIn) -> CanopyOut {
  var position = in.position;
  let edge = world.terrain.xy;
  let inside = clamp(position.xz, -edge, edge);

  if (any(inside != position.xz)) {
    position = vec3<f32>(inside.x, terrain_height_at(inside), inside.y);
  }

  let texel = cover_at(position.xz);
  let red = texel.r * 255.0;
  let crowns = select(0.0, frame.vegetation3.x * red * red * (4.0 / 65025.0), over_terrain(position.xz));
  let species = u32(round(texel.g * 255.0));
  // How far the ground rises above this vertex's canopy edges
  // (`terrain_mesh::canopy_lift`), in metres.
  let lift = f32(in.biome.z);
  position.y = position.y + (mix(8.0, 28.0, species_height(species)) + lift) * saturate(crowns * 10.0);

  var out: CanopyOut;
  out.clip_position = frame.view_proj * vec4<f32>(position, 1.0);
  out.world_position = position;
  // The crown tops follow the ground, softened towards level.
  out.normal = normalize(mix(decode_normal(in.normal), vec3<f32>(0.0, 1.0, 0.0), 0.5));
  out.crowns = crowns;
  out.species = min(species, 7u);
  out.climate = in.climate.xy;
  return out;
}

@fragment
fn canopy_fragment_main(in: CanopyOut) -> @location(0) vec4<f32> {
  let position = in.world_position;
  let distance = length(position - frame.camera_position.xyz);
  let canopy_distance = frame.vegetation3.y;
  // The shell fades in crown by crown as the trees drop out: each 9 m
  // cell from its own hash, anchored to the ground so no screen pattern
  // shows.
  let fade = hash12(floor(position.xz / 9.0) + 3.71);

  if (distance < canopy_distance * 0.8 + fade * canopy_distance * 0.2
    || beyond_render_distance(distance)) {
    discard;
  }

  let view = normalize(frame.camera_position.xyz - position);
  let facing = max(abs(dot(view, in.normal)), 0.05);
  let side = 0.11 * mix(8.0, 28.0, species_height(in.species));
  let seen = 1.0 - exp(-in.crowns * (1.0 + side * sqrt(1.0 - facing * facing) / facing));
  var clump = 0.57;
  // Above 0 where the ground shows between crowns.
  var gap = 0.0;

  if (clumps_unresolved(distance)) {
    // Crowns too small to see: world-anchored cells about two pixels
    // across (9 m doubled as often as needed), each a crown with
    // probability `cover`, so distant forest keeps the speckle of its
    // trees without crawling as the camera moves.
    let level = exp2(ceil(log2(distance * frame.camera_forward.w * frame.viewport.w / 4.5)));
    let cell = floor(position.xz / (9.0 * level)) + level * 7.13;
    gap = select(1.0, 0.0, hash12(cell) < seen);
    clump = 0.2 + 0.8 * hash12(cell + 19.7);
  } else {
    // Seen edge-on, over a ridge, the crowns close up as real ones do
    // (`seen` nears 1), so a skyline is a band of foliage, not a net.
    clump = canopy_clumps(position.xz);
    gap = 1.0 - clump - sqrt(seen);
  }

  if (gap > 0.0) {
    discard;
  }

  let moisture = in.climate.x;
  let temperature = in.climate.y;
  let dryness = saturate((temperature - 0.45) * 1.5 + (0.5 - moisture) * 1.5);
  // The species' impostors, averaged, so the far canopy matches the near
  // trees.
  var albedo = world.species_canopy[in.species].rgb;
  albedo = mix(albedo, albedo * vec3<f32>(1.25, 1.05, 0.55), dryness * 0.35);
  // Crowns stand above the gaps between them: the gaps are in shade.
  let shade = 0.45 + 0.55 * clump;
  let normal = normalize(in.normal + vec3<f32>(0.0, clump * 0.6, 0.0));
  let sun = sun_dir();
  let visibility = cloud_shadow(position) * terrain_shadow(position);
  let wrap = saturate((dot(normal, sun) + 0.45) / 1.45);
  let direct = sun_light() * visibility * wrap * 0.85 * shade;
  // The crowns face the sky: the upward sky light, dimmed as they tilt,
  // costs one sky evaluation instead of two.
  let ambient = sky_irradiance(vec3<f32>(0.0, 1.0, 0.0)) * (0.55 + 0.2 * normal.y) * shade;
  let colour = albedo * (direct + ambient) / PI * 2.6;
  return vec4<f32>(colour, 1.0);
}

// Wet earth at the water's edge: dark, but still brown.
const WET_SOIL: vec3<f32> = vec3<f32>(0.11, 0.085, 0.06);

// --- Banks ---------------------------------------------------------------
//
// Banks beside streams narrower than a heightmap sample (`BankShape` in
// `render/water.rs`), drawn over the terrain's trench: from the water
// outwards, a wet margin of mud, sand or gravel by the flow speed, coming
// and going in patches where grass grows to the water; a face up to the
// bank's top, on a cut bank a near-vertical wall of wet earth with roots,
// on an inner bank a gentle shelf; the dark underside of the turf lip
// that overhangs a cut bank; and the turf, fading into the ground's own
// grass at the back. The water's edge wanders as `water.wgsl` draws it,
// and the bank follows. Banks fade out between 60 and 150 m, and under
// snow and ice, where the terrain's own snow shows.

struct BankIn {
  @location(0) position: vec3<f32>,
  @location(1) outward: vec2<f32>,
  // x: where across the profile (0 the water's edge to 4 the turf's
  // back), y: metres along the centreline, z: speed, w: side (`across`).
  @location(2) params: vec4<f32>,
  // x: how much of a cut bank (0 to 1), y: the stream's width.
  @location(3) shape: vec2<f32>,
};

struct BankOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) world_position: vec3<f32>,
  @location(1) outward: vec2<f32>,
  @location(2) profile: f32,
  @location(3) speed: f32,
  @location(4) cut: f32,
};

@vertex
fn vertex_bank(in: BankIn) -> BankOut {
  var out: BankOut;
  // The ribbon's edge wanders in by up to a fifth of its half width,
  // about four widths along (`water.wgsl`): its visible edge, a little
  // past half the width, moves in by 0.11 w, and so does the bank.
  let width = in.shape.y;
  let footprint = distance(in.position, frame.camera_position.xyz) * frame.camera_forward.w * 2.0 * frame.viewport.w;
  let scale = 1.0 / (4.0 * max(width, 1.0));
  let lod = max(log2(footprint * 512.0 * scale), 0.0);
  let wander = textureSampleLevel(noise_texture, linear_sampler, vec2<f32>(in.params.y * scale, 0.31 + 0.5 * step(0.0, in.params.w)), lod).r;
  let inward = 0.11 * width * smoothstep(0.3, 0.7, wander);
  let position = in.position - vec3<f32>(in.outward.x, 0.0, in.outward.y) * inward;
  out.clip_position = frame.view_proj * vec4<f32>(position, 1.0);
  out.world_position = position;
  out.outward = in.outward;
  out.profile = in.params.x;
  out.speed = in.params.z;
  out.cut = in.shape.x;
  return out;
}

@fragment
fn fragment_bank(in: BankOut) -> @location(0) vec4<f32> {
  let position = in.world_position;
  let ddx_p = dpdx(position);
  let ddy_p = dpdy(position);
  let distance = length(position - frame.camera_position.xyz);
  let view = (frame.camera_position.xyz - position) / max(distance, 0.001);
  // Each facet's own slope, seen from either side.
  var normal = normalize(cross(ddy_p, ddx_p));
  normal = select(-normal, normal, dot(normal, view) >= 0.0);
  let p = in.profile;
  // Shares of the margin, the face, the lip's underside and the turf.
  let face = smoothstep(0.9, 1.1, p) * (1.0 - smoothstep(1.9, 2.1, p));
  let under = smoothstep(1.9, 2.1, p) * (1.0 - smoothstep(2.9, 3.0, p));
  let turf = smoothstep(2.9, 3.0, p);
  let margin = 1.0 - smoothstep(0.9, 1.1, p);
  let along = dot(position.xz, vec2<f32>(-in.outward.y, in.outward.x));
  // Mud where the water is slow, sand, then gravel at 1 m/s and over.
  let sandy = smoothstep(0.3, 0.5, in.speed);
  let gravelly = smoothstep(0.9, 1.1, in.speed);
  let far_blend = smoothstep(35.0, 260.0, distance);
  let mud = sample_material(MAT_MUD, position, ddx_p, ddy_p, normal, far_blend);
  let sand = sample_material(select(MAT_SAND, MAT_GRAVEL, gravelly > 0.5), position, ddx_p, ddy_p, normal, far_blend);
  // The turf is the terrain's own grass: both of its scales, its climate
  // tint, its bankside greening and its broad variation, so it meets the
  // ground with no seam.
  let grass = sample_material(MAT_LUSH, position, ddx_p, ddy_p, vec3<f32>(0.0, 1.0, 0.0), far_blend);
  let coarse = world.material_tints[MAT_SAND].rgb * (1.0 - gravelly) + world.material_tints[MAT_GRAVEL].rgb * gravelly;
  let sediment = mix(mud.albedo * world.material_tints[MAT_MUD].rgb, sand.albedo * coarse, sandy);
  // Grass reaches the water along much of a small stream: the bare, wet
  // margin comes and goes in stretches ten or twenty metres long; a cut
  // bank's narrow margin is bare.
  let patch_scale = 0.07;
  let patches = textureSampleGrad(noise_texture, linear_sampler, position.xz * patch_scale + vec2<f32>(0.41, 0.17), ddx_p.xz * patch_scale, ddy_p.xz * patch_scale).r;
  let bare = max(smoothstep(0.35, 0.65, patches), in.cut);
  let surface = surface_at(position.xz);
  let lush = mix(vec3<f32>(1.12, 0.98, 0.72), vec3<f32>(0.82, 1.0, 0.86), saturate(surface.g * 1.3 - 0.15));
  let fresh = water_banks_at(position.xz).y;
  let broad = textureSampleGrad(noise_texture, linear_sampler, position.xz / 380.0, ddx_p.xz / 380.0, ddy_p.xz / 380.0);
  var turf_albedo = grass.albedo * world.material_tints[MAT_LUSH].rgb * mix(vec3<f32>(1.0), lush, 0.6)
    * mix(vec3<f32>(1.0), vec3<f32>(0.94, 1.08, 0.86), fresh);
  turf_albedo = turf_albedo * (0.84 + broad.g * 0.32) * mix(vec3<f32>(1.0), vec3<f32>(1.05, 1.0, 0.9), broad.b * 0.5);
  // A brook across a wider river's bed, on its gravel, sand or mud: its
  // banks are that bed, not turf.
  let river = channel_field_at(position.xz);
  let on_bed = river.held * (1.0 - river.meshed) * (1.0 - smoothstep(0.6 * river.band, river.band, river.distance))
    * step(0.01, river.band);
  turf_albedo = mix(turf_albedo, sediment * 0.8, on_bed);
  // Wet earth with pale roots hanging down the face: fibres a few
  // centimetres across, streaked down the fall line.
  let root_scale = vec2<f32>(5.0, 0.8);
  let root_uv = vec2<f32>(along, position.y) * root_scale;
  let fibres = textureSampleGrad(noise_texture, linear_sampler, root_uv, vec2<f32>(dot(ddx_p.xz, vec2<f32>(-in.outward.y, in.outward.x)), ddx_p.y) * root_scale, vec2<f32>(dot(ddy_p.xz, vec2<f32>(-in.outward.y, in.outward.x)), ddy_p.y) * root_scale);
  let roots = smoothstep(0.6, 0.7, fibres.r) * smoothstep(0.3, 0.6, fibres.g);
  let earth = mix(WET_SOIL * (1.2 + 0.6 * fibres.b), vec3<f32>(0.32, 0.24, 0.15), roots * 0.7);
  var albedo = mix(turf_albedo, mix(sediment * 0.7, WET_SOIL, 0.35), bare) * margin;
  // An inner bank's shelf is turf down to its margin.
  albedo = albedo + mix(turf_albedo, earth, in.cut) * face;
  albedo = albedo + earth * 0.45 * under + turf_albedo * turf;
  // Under the lip, and at the foot of a cut face, sky light hardly
  // reaches.
  let ao = 1.0 - 0.75 * under - 0.3 * face * in.cut;
  let wet = margin * (1.0 - 0.6 * smoothstep(0.0, 1.0, p)) + face * in.cut * 0.5;
  let colour = shade_surface(albedo, normal, position, ao, mix(0.35, 1.0, wet) * 0.65 * (1.0 - turf), mix(0.6, 0.3, wet));
  let snow = max(snow_cover_at(position.xz), settled_snow_at(position.xz));
  // Where the bank is turf (its top, an inner bank's shelf, a grassy
  // margin), the ground's own surface shows through, whatever it is
  // there; the turf gives way to it entirely towards its back.
  let ground_share = saturate(turf + face * (1.0 - in.cut) + margin * (1.0 - bare));
  let alpha = (1.0 - smoothstep(3.0, 3.6, p)) * mix(1.0, 0.25, ground_share) * (1.0 - smoothstep(60.0, 150.0, distance))
    * (1.0 - snow);

  // The pass writes depth: what hardly shows, ground seen through it and
  // banks faded out, draws nothing.
  if (alpha < 0.2) {
    discard;
  }

  return vec4<f32>(colour, alpha);
}
