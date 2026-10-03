// Draws grass tufts near the camera as two static, world-oriented crossed
// quads per instance (12 vertices, 90 degrees apart), plus a per-instance
// random rotation so a whole meadow does not look axis-aligned. Beyond
// 15 m a tuft is one card turned to the camera (6 vertices), a little
// wider, which covers the same ground on screen for half the vertices.
// Reeds keep three quads, 60 degrees apart (18 vertices). `common.wgsl`
// is prepended.
//
// Each quad is cut into several tapered blades in the fragment shader, so
// one tuft reads as a clump of individual blades. Colour follows the
// local climate (lush green to savannah straw), blades are alpha-tested
// and write depth so they sort correctly, and tufts thin out with a
// screen-space dither near `grassViewDistanceMetres` instead of popping.
// Reeds (style 1) beside still water are the same crossed quads, 1.4 to
// 2.2 m tall and narrow, cut into a few straight stems, some with brown
// seed heads, in the same wind. Under canopy, ferns (style 2) splay each
// quad into a pair of arching fronds, and undergrowth (style 3) is a low,
// wide leafy clump; both take their shapes from the flora texture. The
// style's fraction widens tufts thinned with distance (`grass_generate`).
//
// Light is worked out per vertex: sun and shadow, sky light and the
// glow of back-lit blades. A blade is far smaller than the shadow map's
// texels and the sky's gradients, and a meadow is many layers deep on
// screen, so per pixel it would cost many times as much for no visible
// gain. Each pixel keeps only the blade's shape and colour.

// Flora texture layers for ferns and undergrowth (`tree_models::layers`).
const FERN_LAYER: i32 = 10;
const UNDERGROWTH_LAYER: i32 = 11;
// `CARD_METRES` in `grass_generate.wgsl`.
const CARD_METRES: f32 = 15.0;

struct VertexIn {
  @builtin(vertex_index) vertex_index: u32,
  @location(0) local_offset: vec2<f32>,
  @location(1) uv: vec2<f32>,
  @location(2) instance_position: vec3<f32>,
  @location(3) instance_scale: f32,
  @location(4) instance_tint: f32,
  @location(5) instance_dryness: f32,
  @location(6) instance_style: f32,
};

struct VertexOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) @interpolate(flat) tint: f32,
  @location(2) world_position: vec3<f32>,
  @location(3) @interpolate(flat) fade: f32,
  @location(4) @interpolate(flat) dryness: f32,
  @location(5) normal: vec3<f32>,
  // Settled snow at the tuft: the weather's, or snow lying all year.
  @location(6) @interpolate(flat) snow: f32,
  // 0 tuft, 1 reed, 2 fern, 3 undergrowth.
  @location(7) @interpolate(flat) kind: f32,
  // Which of the quads this is; 2 for a card.
  @location(8) @interpolate(flat) quad: f32,
  // Direct sunlight after shadows, and diffuse sky light, at the vertex.
  @location(9) sun: vec3<f32>,
  @location(10) sky: vec3<f32>,
  // x: how far the sun lights the face, y: the back-lit glow.
  @location(11) light: vec2<f32>,
};

@vertex
fn vertex_main(in: VertexIn) -> VertexOut {
  let world_up = vec3<f32>(0.0, 1.0, 0.0);
  let position_seed = in.instance_position.x * 0.053 + in.instance_position.z * 0.091;
  let kind = floor(in.instance_style + 0.0005);
  let reed = kind == 1.0;
  let to_camera = frame.camera_position.xz - in.instance_position.xz;
  let card = !reed && dot(to_camera, to_camera) > CARD_METRES * CARD_METRES;
  let spacing = select(1.5707963, 1.0471976, reed);
  let base_angle = hash11(position_seed) * spacing;
  var quad_index = f32(in.vertex_index / 6u);
  let quad_angle = base_angle + quad_index * spacing;
  var right = vec3<f32>(cos(quad_angle), 0.0, sin(quad_angle));

  if (card) {
    let across = normalize(vec2<f32>(-to_camera.y, to_camera.x) + vec2<f32>(1e-4, 0.0));
    right = vec3<f32>(across.x, 0.0, across.y);
    quad_index = 2.0;
  }

  // The whole blade bends, more at the tip, with travelling gusts.
  let t = time_seconds();
  var gust = 0.6 + 0.4 * sin(dot(in.instance_position.xz, vec2<f32>(0.8, 0.6)) * 0.05 - t * 1.4);
  var wind = normalize(vec3<f32>(0.8, 0.0, 0.6));

  // The weather's wind: the gust front the trees share, from its
  // direction.
  if (frame.gust.w > 0.5) {
    gust = 0.6 * (1.0 + gust_front(in.instance_position.xz) * frame.gust.y * 0.8);
    let heading = normalize(frame.weather2.zw + vec2<f32>(0.00001, 0.0));
    wind = vec3<f32>(heading.x, 0.0, heading.y);
  }

  let sway_phase = hash11(position_seed * 1.741) * TAU;
  let sway = (sin(t * 2.4 + sway_phase) * 0.35 + 0.65) * gust * frame.vegetation.x * in.uv.y * in.uv.y * 0.9;

  // A riparian tuft carries the eighth of a turn towards the water it
  // leans over as `2 x (sector + 1)` on its dryness.
  let lean_code = floor(in.instance_dryness * 0.5);
  let dryness = in.instance_dryness - 2.0 * lean_code;
  let lean_angle = (lean_code - 1.0) * (TAU / 8.0);
  let width = 1.0 + fract(in.instance_style + 0.0005) * 4.0;
  var height = select(in.instance_scale * mix(1.0, 1.6, dryness), in.instance_scale, reed);
  var spread = select(in.instance_scale * 1.3 * width, 0.6, reed);
  var across = in.local_offset.x;

  if (kind == 2.0) {
    // A pair of fronds from one crown, arching out and down.
    spread = in.instance_scale * 1.7 * width;
    across = in.local_offset.x * mix(0.12, 1.0, in.local_offset.y);
    height = in.instance_scale * (0.75 - 0.3 * in.local_offset.y * in.local_offset.y);
  } else if (kind == 3.0) {
    spread = in.instance_scale * 1.8 * width;
  }

  let lean = select(vec3<f32>(0.0), vec3<f32>(cos(lean_angle), -0.3, sin(lean_angle)), lean_code > 0.5)
    * in.uv.y * in.uv.y * height * 0.7;
  let world_position = in.instance_position
    + right * across * spread
    + world_up * in.local_offset.y * height
    + wind * sway * in.instance_scale
    + lean;

  let view_distance = max(frame.vegetation.w, 1.0);
  let distance_to_camera = length(frame.camera_position.xyz - in.instance_position);
  let fade = 1.0 - smoothstep(view_distance * 0.7, view_distance, distance_to_camera);

  var out: VertexOut;
  out.clip_position = frame.view_proj * vec4<f32>(world_position, 1.0);
  out.uv = in.uv;
  out.tint = in.instance_tint;
  out.world_position = world_position;
  out.fade = fade;
  out.dryness = dryness;
  // Grass normals lean towards up so tufts shade like the ground they
  // grow from rather than like vertical cards.
  out.normal = normalize(world_up * 2.0 + cross(right, world_up) * 0.5);
  out.snow = max(settled_snow_at(in.instance_position.xz), permanent_snow_at(in.instance_position.xz));
  out.kind = kind;
  out.quad = quad_index;
  let sun = sun_dir();
  let view = normalize(frame.camera_position.xyz - world_position);
  out.sun = sun_light() * sun_visibility(world_position, out.normal);
  // The normals lean up so far that the sky seen overhead is all that
  // counts, which spares a second evaluation of the sky model.
  out.sky = sky_irradiance(world_up) * 0.75;
  out.light = vec2<f32>(saturate(dot(out.normal, sun)) * 0.8, pow(saturate(dot(-view, sun)), 3.0));
  return out;
}

@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
  // Gradients first, in uniform control flow: the flora texture is
  // sampled after per-pixel discards.
  let texture_uv = vec2<f32>(in.uv.x, 1.0 - in.uv.y);
  let duv_dx = dpdx(texture_uv);
  let duv_dy = dpdy(texture_uv);

  // Settled snow buries the grass rather than sitting on top of it.
  if (in.fade * (1.0 - in.snow * 0.95) <= pixel_dither(in.clip_position.xy)) {
    discard;
  }

  if (in.kind > 1.5) {
    let layer = select(UNDERGROWTH_LAYER, FERN_LAYER, in.kind < 2.5);
    let texel = textureSampleGrad(flora_texture, linear_sampler, texture_uv, layer, duv_dx, duv_dy);

    // Ferns have five to seven fronds: a third drop one, and a third
    // show the young, coiled frond in the middle.
    let young = abs(in.uv.x - 0.5) < 0.06 && in.uv.y < 0.3;
    let dropped = in.kind < 2.5 && in.quad < 0.5 && in.uv.x < 0.5 && in.tint < 0.33;
    let hidden = in.kind < 2.5 && young && (in.quad > 0.5 || in.tint < 0.67);

    if (texel.a < 0.45 || dropped || hidden) {
      discard;
    }

    let albedo = srgb_to_linear(texel.rgb) * (0.8 + in.tint * 0.4);
    let occlusion = 0.45 + 0.55 * in.uv.y;
    let direct = in.sun * (in.light.x + in.light.y * 0.4);
    let colour = albedo * (direct + in.sky) * occlusion / PI * 2.6;
    return vec4<f32>(colour, 1.0);
  }

  // Seven tapered blades per quad, each with its own height and lean;
  // reeds are three straighter, thinner stems.
  let reed = in.kind > 0.5;
  let blades = select(7.0, 3.0, reed);
  let cell = floor(in.uv.x * blades);
  let local_x = fract(in.uv.x * blades) - 0.5;
  let blade_seed = hash11(cell * 7.13 + in.tint * 31.0);
  let blade_height = select(0.55 + blade_seed * 0.45, 0.75 + blade_seed * 0.25, reed);
  let lean = (blade_seed - 0.5) * select(0.6, 0.15, reed) * in.uv.y;
  let v = in.uv.y / blade_height;
  // A brown seed head near the top of one reed stem in three.
  let head = select(0.0, step(blade_seed, 0.33) * step(0.72, v) * step(v, 0.88), reed);
  let half_width = select(0.42 * (1.0 - v), 0.14 * (1.0 - v * 0.7) + head * 0.14, reed);

  if (v > 1.0 || abs(local_x - lean) > half_width) {
    discard;
  }

  var albedo = mix(blade_base(in.dryness), blade_tip(in.dryness), pow(v, 0.8)) * (0.8 + blade_seed * 0.4) * (0.85 + in.tint * 0.3);

  if (reed) {
    albedo = mix(mix(vec3<f32>(0.05, 0.07, 0.025), vec3<f32>(0.2, 0.22, 0.08), v), vec3<f32>(0.16, 0.09, 0.035), head)
      * (0.85 + in.tint * 0.3);
  }

  let occlusion = 0.35 + 0.65 * v;
  let direct = in.sun * (in.light.x + in.light.y * 0.5 * v);
  let colour = albedo * (direct + in.sky) * occlusion / PI * 2.6;
  return vec4<f32>(colour, 1.0);
}
