// Draws the grown trees. `common.wgsl` is prepended.
//
// Four entry-point pairs share this file, and one module:
//
// - `vertex_mesh`/`fragment_mesh`: the species meshes (see
//   `render/tree_growth.rs`) drawn instanced near the camera. `LIGHT`
//   selects the lighter pipeline for LOD1, which skips the leaves' normal
//   maps and flutter.
// - `vertex_impostor`/`fragment_impostor`: distant trees drawn as a card
//   facing the camera, textured with the two nearest of nine views baked
//   from the very same variant: eight around, one from 40 degrees above.
// - `vertex_shadow`/`fragment_shadow`: every shadow-casting tree as one
//   quad turned to the sun, cut to the view of its impostor nearest the
//   sun, into the tree shadow map.
// - `vertex_bake`/`fragment_bake`: renders each variant's nine views, with
//   their normals, into the impostor atlas once, at start-up.
//
// Instance data comes from the GPU culling pass (`tree_cull.wgsl`), which
// packs `species | variant << 3 | lean << 5` plus half the fade into one
// float, negated where the draw takes the complement of the dither: mesh
// and impostor, and full and light meshes, cross-fade with complementary
// dithers, which never leaves holes.
//
// Leaves take their sunlight after shadows, and their sky light, from
// their card's corners: a leaf card is smaller than the shadow map's
// texels, and a crown is many cards deep on screen, so per pixel it
// would cost many times as much for no visible gain. Bark and impostors
// take the sky light, and the sun through clouds and past hills, from
// their vertices too, as those change slowly across them, but keep the
// trees' shadows per pixel: trunks are tall and impostors large, and
// those shadows sharp. Lying snow is looked up once per tree, and settles
// by the geometric normal of a card or branch, not the bent one, so
// crowns take snow on their upper faces instead of turning white.

// LOD1: no leaf normal maps and no flutter.
override LIGHT: bool = false;

struct MeshIn {
  @location(0) position: vec3<f32>,
  // xyz: normal, w: branch level / 4 plus 0.2 x clump colour (or a knee).
  @location(1) normal: vec4<f32>,
  @location(2) uv: vec2<f32>,
  // x: layer, y: stiffness, z: occlusion, w: branch phase.
  @location(3) params: vec4<f32>,
  @location(11) pivot: vec3<f32>,
  @location(4) instance_position: vec3<f32>,
  @location(5) instance_scale: f32,
  @location(6) instance_rotation: f32,
  @location(7) instance_tint: f32,
  @location(8) instance_species_fade: f32,
  @location(9) instance_dryness: f32,
  // Extra crown width of a thinned far tree (1 for most).
  @location(10) instance_width: f32,
};

struct MeshOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) world_position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) uv: vec2<f32>,
  @location(3) @interpolate(flat) layer: i32,
  @location(4) ao: f32,
  @location(5) @interpolate(flat) species: u32,
  // Fade, negated for the complement of the dither.
  @location(6) @interpolate(flat) fade: f32,
  @location(7) @interpolate(flat) tint: f32,
  @location(8) @interpolate(flat) dryness: f32,
  // Direct sunlight after the clouds and hills (and the trees, for
  // leaves), and diffuse sky light.
  @location(9) sun: vec3<f32>,
  @location(10) sky: vec3<f32>,
  // Snow lying on the tree: the weather's, or snow lying all year.
  @location(11) @interpolate(flat) snow: f32,
  // A clump's colour variation, 0 to 1.
  @location(12) colour: f32,
  // How wet the rain has left the tree, 0 to 1.
  @location(13) @interpolate(flat) wet: f32,
};

// `tree_models::PREVAILING_WIND`, normalised.
const PREVAILING_WIND: vec2<f32> = vec2<f32>(0.8, 0.6);
// `tree_models::WIND_BOUND`.
const WIND_BOUND: f32 = 0.08;
// The impostor atlas: 3 x 3 cells, each with this margin around its view
// so mips never bleed between views (`gpu.rs`).
const ATLAS_MARGIN: f32 = 0.085;
const TOP_TAN: f32 = 0.8390996;

fn rotate_y(v: vec3<f32>, angle: f32) -> vec3<f32> {
  let c = cos(angle);
  let s = sin(angle);
  return vec3<f32>(v.x * c + v.z * s, v.y, -v.x * s + v.z * c);
}

// The drawn code: species, variant and lean.
fn unpack_code(value: f32) -> u32 {
  return u32(floor(abs(value) + 0.0001));
}

// The fade, negated for the complement of the dither.
fn unpack_fade(value: f32) -> f32 {
  let fade = saturate(fract(abs(value) + 0.0001) * 2.0);
  return select(fade, -fade, value < 0.0);
}

// Whether a pixel with this dither is drawn at `fade` (see `unpack_fade`).
fn dithered_out(dither: f32, fade: f32) -> bool {
  return select(dither > fade, dither <= -fade, fade < 0.0);
}

// The wind across the trees: the weather's, or the prevailing wind.
fn wind_direction() -> vec2<f32> {
  let wind = frame.weather2.zw;
  let speed = length(wind);
  return select(normalize(PREVAILING_WIND), wind / max(speed, 0.001), speed > 0.1);
}

// Gusts travel downwind across the forest: the weather's gust front,
// which the grass, rain and water share, or without it a steady pattern.
fn gust_at(instance_xz: vec2<f32>, direction: vec2<f32>, t: f32) -> f32 {
  if (frame.gust.w > 0.5) {
    return 0.55 * (1.0 + gust_front(instance_xz) * frame.gust.y * 0.8);
  }

  let phase = dot(instance_xz, direction) * 0.012 - t * 0.9;
  return 0.55 + 0.45 * sin(phase) * sin(phase * 0.37 + 1.3);
}

// The trunk's bend at `y` metres of a tree `h` tall, from its base, at
// 0.2 to 0.5 Hz: `tree_models::wind_offset`'s first layer.
fn trunk_sway(instance_xz: vec2<f32>, y: f32, h: f32, bend: f32) -> vec3<f32> {
  let w = saturate(frame.vegetation.x) * 1.6;
  let direction = wind_direction();
  let t = time_seconds();
  let frequency = clamp(1.6 / sqrt(h), 0.2, 0.5);
  let sway = 0.7 + 0.3 * sin(TAU * frequency * t + instance_xz.x * 0.13 + instance_xz.y * 0.11);
  let amount = bend * w * w * h * h * 0.0025 * gust_at(instance_xz, direction, t) * sway
    * (y / h) * (y / h);
  return vec3<f32>(direction.x, -0.08, direction.y) * amount;
}

// The layered wind, a port of `tree_models::wind_offset`: the trunk bends,
// limbs bob about their pivots and leaves flutter about their twigs.
// Positions are around the tree's base, after its rotation and scale.
fn wind_offset(
  instance_xz: vec2<f32>,
  position: vec3<f32>,
  pivot: vec3<f32>,
  normal: vec3<f32>,
  h: f32,
  level: f32,
  stiffness: f32,
  phase: f32,
  along: f32,
  leaf: bool,
  species: vec2<f32>
) -> vec3<f32> {
  let w = saturate(frame.vegetation.x) * 1.6;

  if (w <= 0.0001) {
    return vec3<f32>(0.0);
  }

  let direction = wind_direction();
  let t = time_seconds();
  let height = max(h, 0.1);
  let y = max(position.y, 0.0);
  let gust = gust_at(instance_xz, direction, t);
  var d = trunk_sway(instance_xz, y, height, species.x);
  let rise = saturate(y / (0.15 * height));
  let tree_phase = instance_xz.x * 0.13 + instance_xz.y * 0.11;

  if (level >= 0.5) {
    let frequency = 0.8 + 0.7 * fract(phase * 0.159);
    let angle = w * (1.0 - saturate(stiffness)) * 0.1 * gust * sin(TAU * frequency * t + phase + tree_phase);
    let bob = angle * length(position - pivot) * rise / sqrt(1.25);
    d = d + vec3<f32>(direction.x * 0.5, 1.0, direction.y * 0.5) * bob;
  }

  if (leaf && !LIGHT) {
    let frequency = 3.0 + 3.0 * fract(phase * 0.37 + 0.5);
    let flutter = w * species.y * 0.12 * sin(TAU * frequency * t + phase * 3.0 + position.y * 1.7) * along * rise;
    d = d + normal * flutter;
  }

  let moved = length(d);
  let bound = WIND_BOUND * height;
  return select(d, d * (bound / max(moved, 0.0001)), moved > bound);
}

// The lean (bits 5 to 10 of the drawn code): up to 6 degrees, pivoting
// at the base so roots stay grounded, and on slopes a pistol-butt bend
// of the lowest fifth of the trunk that then grows back upright.
fn lean_offset(code: u32, y: f32, h: f32) -> vec3<f32> {
  let lean = code >> 5u;
  let angle = f32(lean & 15u) * (TAU / 16.0);
  let strength = f32((lean >> 4u) & 3u);
  let direction = vec2<f32>(sin(angle), cos(angle));
  let low = 0.2 * h;
  let y0 = clamp(y, 0.0, low);
  let butt = tan(radians(max(strength - 1.0, 0.0) * 8.0)) * (y0 - y0 * y0 / (2.0 * low));
  let amount = max(y, 0.0) * tan(radians(strength * 2.0)) + butt;
  return vec3<f32>(direction.x, 0.0, direction.y) * amount;
}

@vertex
fn vertex_mesh(in: MeshIn) -> MeshOut {
  let code = unpack_code(in.instance_species_fade);
  let species = code & 7u;
  let bounds = world.species[species];
  let scale = in.instance_scale;
  let tree_height = bounds.x * scale;
  let packed = in.normal.w * 4.0;
  let level = floor(packed + 0.01);
  let extra = saturate((packed - level) / 0.8);
  let layer = i32(in.params.x + 0.5);
  let leaf = layer >= 4;
  var model = vec3<f32>(in.position.x * in.instance_width, in.position.y, in.position.z * in.instance_width);

  // A swamp cypress's knees sink into dry ground.
  if (!leaf && extra > 0.5) {
    model.y = model.y - 1.5 * smoothstep(0.3, 0.5, in.instance_dryness);
  }

  var local = rotate_y(model * scale, in.instance_rotation);
  local = local + lean_offset(code, local.y, tree_height);
  let normal = rotate_y(in.normal.xyz, in.instance_rotation);
  let pivot = rotate_y(in.pivot * scale, in.instance_rotation);
  let offset = wind_offset(
    in.instance_position.xz,
    local,
    pivot,
    normal,
    tree_height,
    level,
    in.params.y,
    in.params.w,
    in.uv.y,
    leaf,
    bounds.zw
  );
  let world_position = in.instance_position + local + offset;

  var out: MeshOut;
  out.clip_position = frame.view_proj * vec4<f32>(world_position, 1.0);
  out.world_position = world_position;
  out.normal = normal;
  out.uv = in.uv;
  out.layer = layer;
  out.ao = in.params.z;
  out.species = species;
  out.fade = unpack_fade(in.instance_species_fade);
  out.tint = in.instance_tint;
  out.dryness = in.instance_dryness;
  out.sun = sun_light() * cloud_shadow(world_position) * terrain_shadow(world_position);
  out.sky = sky_irradiance(out.normal);
  let ground_weather = surface_weather_at(in.instance_position.xz);
  out.snow = max(ground_weather.z, permanent_snow_at(in.instance_position.xz));
  out.wet = ground_weather.x;
  out.colour = extra;

  if (leaf) {
    out.sun = out.sun * tree_shadow(world_position, out.normal);
  } else {
    // Under a closed canopy little sky reaches a trunk, least at its foot.
    out.sky = out.sky * (1.0 - 0.6 * canopy_at(in.instance_position.xz) * saturate(1.0 - local.y / tree_height));
  }

  return out;
}

// Rain darkens bark and leaves (`wet`, from the ground under the tree);
// settled snow, from the weather or lying all year round, whitens the
// faces turned up (`up`, the geometric normal's height). `cover` is the
// snow lying on the tree.
fn weather_surface(albedo: vec3<f32>, up: f32, cover: f32, wet: f32) -> vec3<f32> {
  var colour = albedo * (1.0 - wet * 0.25);
  let snow = cover * smoothstep(0.1, 0.7, up);
  return mix(colour, vec3<f32>(0.78, 0.82, 0.88), snow * 0.85);
}

// Per tree, hue varies by about 4 %, saturation 10 % and brightness 8 %
// with its tint, and per clump a little more.
fn foliage_colour(albedo: vec3<f32>, species: u32, tint: f32, dryness: f32, clump: f32) -> vec3<f32> {
  let t = tint - 0.5;
  var colour = albedo * world.species_tint[species].rgb;
  let grey = luminance(colour);
  colour = mix(vec3<f32>(grey), colour, 1.0 + 0.2 * t);
  colour = colour * vec3<f32>(1.0 + 0.08 * t, 1.0, 1.0 - 0.08 * t);
  colour = colour * (1.0 + 0.16 * t) * (0.9 + 0.2 * clump);
  // Hot, dry climates bleach and yellow the canopy slightly.
  colour = mix(colour, colour * vec3<f32>(1.25, 1.05, 0.55), dryness * 0.35);
  return colour;
}

// `sun` is the sunlight after shadows, and `sky` the sky light, at the
// leaf.
fn shade_foliage(
  albedo: vec3<f32>,
  normal: vec3<f32>,
  world_position: vec3<f32>,
  occlusion: f32,
  sun_lit: vec3<f32>,
  sky: vec3<f32>
) -> vec3<f32> {
  let sun = sun_dir();
  let view = normalize(frame.camera_position.xyz - world_position);
  // Wrap lighting: light bleeds around the rounded crown.
  let wrap = saturate((dot(normal, sun) + 0.45) / 1.45);
  // Translucency: leaves glow when back-lit by the sun.
  let back = pow(saturate(dot(-view, sun)), 3.0) * 0.6;
  let direct = sun_lit * (wrap * 0.85 + back * vec3<f32>(0.9, 1.1, 0.55));
  return albedo * (direct * occlusion + sky * occlusion * 0.75) / PI * 2.6;
}

// The flora layer holding a leaf layer's normal map:
// `tree_models::layers::normal_layer`.
fn leaf_normal_layer(layer: i32) -> i32 {
  return select(layer + 11, layer + 8, layer >= 12);
}

// Bark takes a moss and lichen tint on its shaded, wet side in wet
// climates.
fn bark_colour(albedo: vec3<f32>, normal: vec3<f32>, tint: f32, dryness: f32) -> vec3<f32> {
  let damp = saturate(1.0 - dryness * 1.6);
  let shaded = saturate(0.5 - dot(normal, sun_dir()) * 0.8);
  let moss = damp * shaded * (0.35 + 0.3 * tint);
  let green = albedo * vec3<f32>(0.75, 1.1, 0.6) + vec3<f32>(0.0, 0.012, 0.0);
  return mix(albedo * (0.9 + tint * 0.2), green, moss);
}

@fragment
fn fragment_mesh(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
  // Derivatives first, while control flow is uniform.
  let dx = dpdx(in.world_position);
  let dy = dpdy(in.world_position);
  let du = dpdx(in.uv);
  let dv = dpdy(in.uv);
  let texel = textureSample(flora_texture, linear_sampler, in.uv, in.layer);
  let leaf = in.layer >= 4;
  var bump = vec2<f32>(0.0);

  if (!LIGHT) {
    bump = textureSample(flora_texture, linear_sampler, in.uv, leaf_normal_layer(select(4, in.layer, leaf))).xy * 2.0 - 1.0;
  }

  // The geometric normal, turned to the camera.
  var geometric = normalize(cross(dx, dy));
  geometric = select(-geometric, geometric, dot(geometric, frame.camera_position.xyz - in.world_position) > 0.0);

  if (dithered_out(pixel_dither(in.clip_position.xy), in.fade)) {
    discard;
  }

  if (leaf) {
    if (texel.a < 0.45) {
      discard;
    }

    var normal = normalize(in.normal);

    if (!LIGHT) {
      // The midrib's relief, in the card's own frame from the screen-space
      // derivatives.
      let determinant = du.x * dv.y - du.y * dv.x;
      let inverse = select(0.0, 1.0 / determinant, abs(determinant) > 1e-12);
      let tangent = (dx * dv.y - dy * du.y) * inverse;
      let bitangent = (dy * du.x - dx * dv.x) * inverse;
      let scale = 0.5 / max(max(length(tangent), length(bitangent)), 1e-4);
      normal = normalize(normal + (tangent * bump.x + bitangent * bump.y) * scale * 0.6);
    }

    var albedo = foliage_colour(srgb_to_linear(texel.rgb), in.species, in.tint, in.dryness, in.colour);
    albedo = weather_surface(albedo, geometric.y, in.snow, in.wet);
    return vec4<f32>(shade_foliage(albedo, normal, in.world_position, in.ao, in.sun, in.sky), 1.0);
  }

  var normal = normalize(in.normal);

  if (!front) {
    normal = -normal;
  }

  let bark = bark_colour(srgb_to_linear(texel.rgb), normal, in.tint, in.dryness);
  // Soaked bark darkens more than leaves.
  let albedo = weather_surface(bark, normal.y, in.snow, min(in.wet * 1.4, 1.0));
  // `shade_surface` without specular, with the sky light from the
  // vertices.
  let direct = in.sun * saturate(dot(normal, sun_dir())) * tree_shadow(in.world_position, normal);
  let colour = albedo * (direct + in.sky * in.ao * (0.6 + texel.a * 0.4) * 0.75) / PI * 2.6;
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
  @location(10) instance_width: f32,
};

struct ImpostorOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) world_position: vec3<f32>,
  // The point in the view drawn (0 to 1 within it; outside it, empty).
  @location(1) at: vec2<f32>,
  // The card's frame: right and towards the camera.
  @location(3) right: vec3<f32>,
  @location(4) facing: vec3<f32>,
  @location(5) @interpolate(flat) fade: f32,
  @location(6) @interpolate(flat) tint: f32,
  @location(7) @interpolate(flat) dryness: f32,
  // Sunlight after the clouds and hills, and diffuse sky light.
  @location(8) sun: vec3<f32>,
  @location(9) sky: vec3<f32>,
  @location(10) @interpolate(flat) snow: f32,
  // x: the view drawn (cell 0 to 8), y: the atlas layer of the variant's
  // colour.
  @location(11) @interpolate(flat) view: vec2<f32>,
  // The atlas mip level to sample.
  @location(12) @interpolate(flat) level: f32,
  // How wet the rain has left the tree, 0 to 1.
  @location(13) @interpolate(flat) wet: f32,
};

// The atlas uv of a point `uv` (0 to 1) in `cell` (0 to 8).
fn atlas_uv(cell: f32, uv: vec2<f32>) -> vec2<f32> {
  let column = cell % 3.0;
  let row = floor(cell / 3.0);
  let inner = ATLAS_MARGIN + saturate(uv) * (1.0 - 2.0 * ATLAS_MARGIN);
  return (vec2<f32>(column, row) + inner) / 3.0;
}

// The view in `cell` at card point `card` (x -1 to 1, y in heights): its
// uv. Side views span the height; the top view spans it and twice the
// radius's rise at 40 degrees, drawn on a plane in front of the trunk.
// `top`: x the radius's rise over the height, y the top view's weight.
fn view_uv(cell: f32, card: vec2<f32>, top: vec2<f32>) -> vec2<f32> {
  let above = cell > 7.5;
  let span = select(1.0, 1.0 + 2.0 * top.x, above);
  let y = card.y + select(0.0, top.x * (1.0 - top.y), above);
  return vec2<f32>(card.x * 0.5 + 0.5, 1.0 - y / span);
}

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
  let code = unpack_code(in.instance_species_fade);
  let species = code & 7u;
  let variant = (code >> 3u) & 3u;
  let bounds = world.species[species];
  let height = bounds.x * in.instance_scale;
  let radius = bounds.y * in.instance_scale * in.instance_width;
  let up = vec3<f32>(0.0, 1.0, 0.0);
  let to_camera = frame.camera_position.xyz - in.instance_position;
  let flat_length = max(length(to_camera.xz), 0.0001);
  var facing = vec3<f32>(to_camera.x / flat_length, 0.0, to_camera.z / flat_length);
  var right: vec3<f32>;
  // Looking down from above, the view from 40 degrees takes over.
  var top = saturate(atan2(to_camera.y, flat_length) / radians(40.0));

  if (frame.vegetation.z > 0.5 && frame.vegetation.z < 1.5) {
    // Crossed quads: two fixed planes per tree, seen from the side.
    let angle = in.instance_rotation + select(0.0, 1.5707963, in.vertex_index >= 6u);
    right = vec3<f32>(cos(angle), 0.0, -sin(angle));
    facing = cross(right, up);
    top = 0.0;
  } else {
    right = normalize(cross(up, facing));
  }

  // The top view stands in front of the trunk and reaches higher: from
  // above, the crown's front hides the ground before the trunk and its
  // back rises above the top.
  let reach = height + 2.0 * radius * TOP_TAN * top;
  let lift = corner.y * reach;
  var world_position = in.instance_position + facing * radius * top + right * corner.x * radius + up * lift;
  world_position = world_position + lean_offset(code, lift, height)
    + trunk_sway(in.instance_position.xz, min(lift, height), max(height, 0.1), bounds.z);

  // The camera's direction in the tree's own frame picks the views.
  let model = rotate_y(facing, -in.instance_rotation);
  let around = fract(atan2(model.x, model.z) / TAU + 1.0) * 8.0;
  let first = floor(around);
  let t = around - first;
  // The two heaviest of: first, second and top.
  var views = vec4<f32>(first, (first + 1.0) % 8.0, t, f32((species * u32(world.trees.x) + variant) * 2u));
  let weights = vec3<f32>((1.0 - t) * (1.0 - top), t * (1.0 - top), top);

  if (weights.z > min(weights.x, weights.y)) {
    let side = select(first, views.y, weights.y > weights.x);
    let kept = max(weights.x, weights.y);
    views = vec4<f32>(side, 8.0, weights.z / max(kept + weights.z, 0.0001), views.w);
  }

  // The mip level whose texels match the card's pixels: a view's height
  // is 160 texels.
  let depth = max(dot(world_position - frame.camera_position.xyz, frame.camera_forward.xyz), 0.1);
  let pixels = reach / (2.0 * frame.camera_forward.w * depth) * frame.viewport.y;
  let card = vec2<f32>(corner.x, lift / max(height, 0.001));
  let rise = vec2<f32>(radius / max(height, 0.001) * TOP_TAN, top);

  var out: ImpostorOut;
  out.clip_position = frame.view_proj * vec4<f32>(world_position, 1.0);
  out.world_position = world_position;
  // Each tree draws one of its two nearest views whole, switching at its
  // own point between them: a forest turns over tree by tree as the
  // camera moves, where a per-pixel blend hatched every crown.
  let second = hash11(in.instance_position.x * 0.7137 + in.instance_position.z * 1.3713) < views.z;
  let cell = select(views.x, views.y, second);
  out.at = view_uv(cell, card, rise);
  out.right = right;
  out.facing = facing;
  out.fade = unpack_fade(in.instance_species_fade);
  out.tint = in.instance_tint;
  out.dryness = in.instance_dryness;
  out.sun = sun_light() * cloud_shadow(world_position) * terrain_shadow(world_position);
  out.sky = sky_irradiance(normalize(facing + up));
  let ground_weather = surface_weather_at(in.instance_position.xz);
  out.snow = max(ground_weather.z, permanent_snow_at(in.instance_position.xz));
  out.wet = ground_weather.x;
  out.view = vec2<f32>(cell, views.w);
  // The atlas has five levels (`IMPOSTOR_MIPS`).
  out.level = clamp(log2(160.0 / max(pixels, 0.001)), 0.0, 4.0);
  return out;
}

@fragment
fn fragment_impostor(in: ImpostorOut) -> @location(0) vec4<f32> {
  let dither = pixel_dither(in.clip_position.xy);
  // Past a view's edge its margin, which is empty, is sampled.
  let at = atlas_uv(in.view.x, in.at);
  let layer = i32(in.view.y);
  let texel = textureSampleLevel(impostor_texture, linear_sampler, at, layer, in.level);

  // Complementary to the mesh dither in the cross-fade band.
  if (texel.a < 0.5 || dithered_out(dither, in.fade)) {
    discard;
  }

  let packed = textureSampleLevel(impostor_texture, linear_sampler, at, layer + 1, in.level).xyz * 2.0 - 1.0;
  let up = vec3<f32>(0.0, 1.0, 0.0);
  let normal = normalize(in.right * packed.x + up * packed.y + in.facing * packed.z);
  var albedo = srgb_to_linear(texel.rgb) * (0.82 + in.tint * 0.36);
  albedo = mix(albedo, albedo * vec3<f32>(1.25, 1.05, 0.55), in.dryness * 0.35);
  albedo = weather_surface(albedo, normal.y + 0.3, in.snow, in.wet);
  let sun_lit = in.sun * tree_shadow(in.world_position, normal);
  let colour = shade_foliage(albedo, normal, in.world_position, 0.9, sun_lit, in.sky);
  return vec4<f32>(colour, 1.0);
}

struct ShadowOut {
  @builtin(position) clip_position: vec4<f32>,
  // In the impostor atlas.
  @location(0) uv: vec2<f32>,
  @location(1) @interpolate(flat) layer: i32,
};

@vertex
fn vertex_shadow(in: ImpostorIn) -> ShadowOut {
  var corners = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(-1.0, 1.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, 1.0)
  );
  let corner = corners[in.vertex_index % 6u];
  let code = unpack_code(in.instance_species_fade);
  let species = code & 7u;
  let variant = (code >> 3u) & 3u;
  let bounds = world.species[species];
  let height = bounds.x * in.instance_scale;
  let sun = sun_dir();
  let flat_length = max(length(sun.xz), 0.0001);
  let facing = vec3<f32>(sun.x / flat_length, 0.0, sun.z / flat_length);
  let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), facing));
  let lift = corner.y * height;
  // Shadows sway and lean with their trees.
  let world_position = in.instance_position
    + right * corner.x * bounds.y * in.instance_scale * in.instance_width
    + vec3<f32>(0.0, lift, 0.0)
    + lean_offset(code, lift, height)
    + trunk_sway(in.instance_position.xz, lift, max(height, 0.1), bounds.z);
  // The side view nearest the sun's direction in the tree's frame.
  let model = rotate_y(facing, -in.instance_rotation);
  let cell = round(fract(atan2(model.x, model.z) / TAU + 1.0) * 8.0) % 8.0;

  var out: ShadowOut;
  out.clip_position = frame.shadow_view_proj * vec4<f32>(world_position, 1.0);
  out.uv = atlas_uv(cell, vec2<f32>(corner.x * 0.5 + 0.5, 1.0 - corner.y));
  out.layer = i32((species * u32(world.trees.x) + variant) * 2u);
  return out;
}

@fragment
fn fragment_shadow(in: ShadowOut) {
  // The impostor's mips keep their alpha coverage, so at a level near
  // the shadow map's own texel size the gaps between leaves stay open
  // and sun flecks reach the forest floor.
  if (textureSampleLevel(impostor_texture, linear_sampler, in.uv, in.layer, 1.0).a < 0.5) {
    discard;
  }
}

struct BakeIn {
  // species x 16 + view.
  @builtin(instance_index) view: u32,
  @location(0) position: vec3<f32>,
  @location(1) normal: vec4<f32>,
  @location(2) uv: vec2<f32>,
  @location(3) params: vec4<f32>,
};

struct BakeOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) @interpolate(flat) layer: i32,
  @location(2) ao: f32,
  // The normal in the card's frame: right, up, towards the camera.
  @location(3) normal: vec3<f32>,
  @location(4) @interpolate(flat) species: u32,
  @location(5) colour: f32,
};

struct BakeTargets {
  @location(0) colour: vec4<f32>,
  @location(1) normal: vec4<f32>,
};

// Each view fills its atlas cell (the pass's viewport): side views are
// orthographic from the horizon, eight around at 45 degrees; the ninth
// looks down from 40 degrees above +z, projected along its rays onto the
// vertical plane in front of the crown, which is where the impostor
// shader draws it.
@vertex
fn vertex_bake(in: BakeIn) -> BakeOut {
  let species = in.view / 16u;
  let view = in.view % 16u;
  let bounds = world.species[species];
  let yaw = f32(min(view, 8u) % 8u) * (TAU / 8.0);
  // Turn the tree so the camera looks from +z.
  let p = rotate_y(in.position, -yaw);
  let n = rotate_y(in.normal.xyz, -yaw);
  let radius = bounds.y;
  var y = p.y / bounds.x;
  var depth = 0.5 - p.z / (4.0 * radius);

  if (view == 8u) {
    y = (p.y + (radius - p.z) * TOP_TAN) / (bounds.x + 2.0 * radius * TOP_TAN);
    depth = 0.5 - (p.z * 0.766 + p.y * 0.643) / (4.0 * max(radius, bounds.x));
  }

  let inner = 1.0 - 2.0 * ATLAS_MARGIN;
  var out: BakeOut;
  out.clip_position = vec4<f32>(p.x / radius * inner, (y * 2.0 - 1.0) * inner, clamp(depth, 0.0, 1.0), 1.0);
  out.uv = in.uv;
  out.layer = i32(in.params.x + 0.5);
  out.ao = in.params.z;
  out.normal = n;
  out.species = species;
  out.colour = saturate((in.normal.w * 4.0 - floor(in.normal.w * 4.0 + 0.01)) / 0.8);
  return out;
}

@fragment
fn fragment_bake(in: BakeOut) -> BakeTargets {
  let texel = textureSample(flora_texture, linear_sampler, in.uv, in.layer);

  if (in.layer >= 4 && texel.a < 0.45) {
    discard;
  }

  var albedo = srgb_to_linear(texel.rgb);

  if (in.layer >= 4) {
    albedo = albedo * world.species_tint[in.species].rgb * (0.9 + 0.2 * in.colour);
  } else {
    albedo = bark_colour(albedo, vec3<f32>(0.0, 0.0, 1.0), 0.5, 0.5);
  }

  // Bake occlusion and a soft top-light into the impostor so it keeps the
  // mesh's depth cues when lit flat.
  let normal = normalize(in.normal);
  let shade = in.ao * (0.78 + 0.22 * saturate(normal.y * 0.5 + 0.5));
  var out: BakeTargets;
  out.colour = vec4<f32>(pow(albedo * shade, vec3<f32>(1.0 / 2.2)), 1.0);
  // Alpha marks foliage (1) from bark (0.5), so the canopy layer's colour
  // can be averaged from the leaves alone (`mipgen.wgsl`).
  out.normal = vec4<f32>(normal * 0.5 + 0.5, select(0.5, 1.0, in.layer >= 4));
  return out;
}
