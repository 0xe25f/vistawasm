// Ground materials, shared by the terrain and the boulders:
// `common.wgsl` comes first.
//
// - Material sampling: each material's baked textures, projected on the
//   world planes, and parallax through their height channels.
// - Bare rock: jointed blocks, bedding ledges, lichen and water streaks.
//
// Everything samples with `textureSampleGrad` and explicit gradients, so
// it is safe inside the per-pixel branches that call it.
//
// `frame.rock`: xy the beds' rise per metre along x and z (their dip,
// `soil::Strata`), z the beds' spacing in metres, w the angle, in
// radians round from +x towards +z, of the side the sun mostly shines
// on, where lichen grows.

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
const MAT_GRAVEL: i32 = 10;
const MAT_SCREE: i32 = 11;
const MATERIAL_COUNT: i32 = 12;

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
    case 10: { return 2.0; }
    case 11: { return 3.0; }
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
    case 11: { return vec3<f32>(0.15, 0.14, 0.13); }
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
// Rock, scree and glacier ice take triplanar weights from the geometric normal,
// as they cover steep slopes, and so does snow as the slope steepens past
// normal.y 0.9 to 0.7, with no seam where the projection changes. The rest
// project from above. Near-zero weights are dropped, so flat ground takes
// one sample, not three.
fn projection_weights(material: i32, normal: vec3<f32>) -> vec3<f32> {
  var steep = select(0.0, 1.0, material == MAT_ROCK || material == MAT_ICE || material == MAT_SCREE);

  if (material == MAT_SNOW) {
    steep = 1.0 - smoothstep(0.7, 0.9, normal.y);
  }

  let sharp = pow(abs(normal), vec3<f32>(4.0));
  let triplanar = sharp / max(sharp.x + sharp.y + sharp.z, 0.0001);
  let weights = max(mix(vec3<f32>(0.0, 1.0, 0.0), triplanar, steep) - 0.02, vec3<f32>(0.0));
  return weights / max(weights.x + weights.y + weights.z, 0.0001);
}

// A vector's coordinates on world plane `axis` (0: ZY, 1: XZ, 2: XY).
fn plane(axis: i32, v: vec3<f32>) -> vec2<f32> {
  return select(select(v.xz, v.xy, axis == 2), v.zy, axis == 0);
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
      let s = sample_planar(material, plane(axis, position) * inv + offset, plane(axis, ddx_p) * inv, plane(axis, ddy_p) * inv);
      result.albedo = result.albedo + s.albedo * w;
      result.height = result.height + s.height * w;
      result.detail = result.detail + s.detail * w;
      result.occlusion = result.occlusion + s.occlusion * w;
      result.roughness = result.roughness + s.roughness * w;
    }
  }

  return result;
}

// A material's height channel on its projection planes, at the near
// scale: what parallax steps through.
fn material_height(material: i32, position: vec3<f32>, ddx_p: vec3<f32>, ddy_p: vec3<f32>, weights: vec3<f32>) -> f32 {
  let inv = 1.0 / (material_scale(material) * max(frame.surface.z, 0.01));
  var height = 0.0;

  for (var axis = 0; axis < 3; axis = axis + 1) {
    if (weights[axis] > 0.0) {
      height = height + textureSampleGrad(terrain_albedo, linear_sampler, plane(axis, position) * inv, material, plane(axis, ddx_p) * inv, plane(axis, ddy_p) * inv).a * weights[axis];
    }
  }

  return height;
}

// Parallax occlusion for rock and scree: step down the material's height
// field (1 at its top) along the view ray, four steps through `depth`
// metres, and return where the ray meets it, so joints and cobbles hide
// what lies behind them.
fn parallax(position: vec3<f32>, ddx_p: vec3<f32>, ddy_p: vec3<f32>, normal: vec3<f32>, material: i32, depth: f32) -> vec3<f32> {
  let view = normalize(frame.camera_position.xyz - position);
  let facing = max(dot(view, normal), 0.25);
  // The ray's drift across the surface per metre it sinks.
  let drift = (normal * dot(view, normal) - view) / facing;
  let weights = projection_weights(material, normal);
  var sunk = 0.0;
  var p = position;

  for (var step = 1; step <= 4; step = step + 1) {
    if (1.0 - material_height(material, p, ddx_p, ddy_p, weights) <= sunk) {
      break;
    }

    sunk = f32(step) * 0.25;
    p = position + drift * depth * sunk;
  }

  return p;
}

struct RockDetail {
  // Dark joints between blocks and along the bedding, 0 to 1.
  joint: f32,
  // The block's own tone, about 0.8 to 1.2.
  tone: f32,
  // Tilt of the block's face, added to the normal: faceted blocks, and
  // ledges whose tops face the sky.
  tilt: vec3<f32>,
};

// A line `width` metres wide at distance `d` metres from its centre,
// averaged over a pixel `footprint` metres across: thin lines fade to
// their mean darkening instead of shimmering as they shrink.
fn rock_line(d: f32, width: f32, footprint: f32) -> f32 {
  return (1.0 - smoothstep(width, width + footprint, d)) * min(width * 2.0 / max(footprint, 0.001), 1.0);
}

// Voronoi cells on a unit lattice: x distance to the nearest point, y to
// the second nearest, z the nearest cell's hash.
fn rock_cells(p: vec2<f32>) -> vec3<f32> {
  let base = floor(p);
  var result = vec3<f32>(8.0, 8.0, 0.0);

  for (var j = -1; j <= 1; j = j + 1) {
    for (var i = -1; i <= 1; i = i + 1) {
      let cell = base + vec2<f32>(f32(i), f32(j));
      let id = hash12(cell);
      let d = length(cell + vec2<f32>(id, hash12(cell + 17.3)) - p);

      if (d < result.x) {
        result = vec3<f32>(d, result.x, id);
      } else if (d < result.y) {
        result.y = d;
      }
    }
  }

  return result;
}

// Macro joints, 6 to 20 m apart. Vertical joint planes are Voronoi
// borders in plan, 9 m cells stretched 1.8 times along the beds' strike,
// so a cliff breaks into tall blocks and a rock pavement into slabs.
// Bedding planes are level lines every quarter of the beds' spacing,
// tilted by their dip and bent by a 400 m noise, so ledges run along the
// contours as the outcrops do. `footprint` is the pixel's size in metres.
fn rock_detail(position: vec3<f32>, ddx_p: vec3<f32>, ddy_p: vec3<f32>, distance: f32) -> RockDetail {
  let footprint = max(length(ddx_p), length(ddy_p));
  let dip = frame.rock.xy;
  let strike = select(vec2<f32>(1.0, 0.0), normalize(vec2<f32>(-dip.y, dip.x)), dot(dip, dip) > 1e-8);
  // A noise about 20 m across bends every joint and bed, so none runs
  // ruler-straight, and hides about half of each bedding line: beds pinch
  // out.
  let bend_scale = 1.0 / 23.0;
  let bend = textureSampleGrad(noise_texture, linear_sampler, (position.xz + position.y * 0.6) * bend_scale, (ddx_p.xz + ddx_p.y * 0.6) * bend_scale, (ddy_p.xz + ddy_p.y * 0.6) * bend_scale);
  let q = vec2<f32>(dot(position.xz, strike) / 1.8, dot(position.xz, vec2<f32>(-strike.y, strike.x))) / 9.0 + (bend.rg - 0.5) * 0.35;
  let cells = rock_cells(q);
  let warp_scale = 1.0 / 400.0;
  let warp = textureSampleGrad(noise_texture, linear_sampler, position.xz * warp_scale, ddx_p.xz * warp_scale, ddy_p.xz * warp_scale).r;
  let spacing = max(frame.rock.z * 0.25, 2.0);
  let phase = (position.y + dot(dip, position.xz)) / spacing + warp * 1.3 + (bend.a - 0.5) * 0.4;
  let bed = abs(fract(phase) - 0.5);
  // Joints narrower than a pixel only darken a little, on average.
  let joints = rock_line((cells.y - cells.x) * 9.0 * 0.5, 0.08, footprint);
  let ledges = rock_line((0.5 - bed) * spacing, 0.07, max(abs(ddx_p.y), abs(ddy_p.y))) * smoothstep(0.35, 0.55, bend.g);
  // Block tone from the noise texture's Worley channel and each block's
  // own hash, so neighbouring blocks weather differently.
  let worley = textureSampleGrad(noise_texture, linear_sampler, q * 0.37, (ddx_p.xz / 9.0) * 0.37, (ddy_p.xz / 9.0) * 0.37).b;
  let block = hash12(vec2<f32>(cells.z * 97.0, floor(phase)));
  // Far away the near texture has averaged out, so the blocks carry the
  // rock's structure: their facets strengthen with distance.
  let strength = mix(0.3, 0.8, smoothstep(50.0, 800.0, distance));
  let facet = vec2<f32>(hash12(vec2<f32>(block, 3.1)), hash12(vec2<f32>(block, 7.9))) - 0.5;
  let ledge_top = smoothstep(0.75, 1.0, fract(phase));
  var detail: RockDetail;
  detail.joint = max(joints, ledges);
  detail.tone = 0.84 + 0.2 * block + 0.14 * worley;
  detail.tilt = vec3<f32>(facet.x, ledge_top * 0.6, facet.y) * 0.5 * strength;
  return detail;
}

// Crustose lichen on moist rock that faces the sun and is gently
// inclined: sparse pale grey-green and ochre patches about 0.4 m across,
// under 20 % of the rock. None in arid or ice-arctic ground. Returns the
// colour (rgb) and how much of it covers the rock (a).
fn rock_lichen(position: vec3<f32>, ddx_p: vec3<f32>, ddy_p: vec3<f32>, normal: vec3<f32>, moisture: f32, arctic: bool) -> vec4<f32> {
  let sunward = vec2<f32>(cos(frame.rock.w), sin(frame.rock.w));
  let facing = saturate(dot(normal.xz, sunward) * 1.5 + 0.35);
  let inclined = smoothstep(0.3, 0.55, normal.y) * (1.0 - smoothstep(0.9, 0.99, normal.y));
  let wet = smoothstep(0.3, 0.55, moisture) * select(1.0, 0.0, arctic);
  let amount = facing * inclined * wet;

  if (amount <= 0.001) {
    return vec4<f32>(0.0);
  }

  // Projected on the plane the face is nearest, so patches keep their
  // size on walls.
  let steep = abs(normal.y) < 0.7;
  let across = select(position.zy, position.xy, abs(normal.z) > abs(normal.x));
  let dx = select(ddx_p.zy, ddx_p.xy, abs(normal.z) > abs(normal.x));
  let dy = select(ddy_p.zy, ddy_p.xy, abs(normal.z) > abs(normal.x));
  let uv = select(position.xz, across, steep) / 6.4;
  let n = textureSampleGrad(noise_texture, linear_sampler, uv, select(ddx_p.xz, dx, steep) / 6.4, select(ddy_p.xz, dy, steep) / 6.4);
  // Patchy crusts, their cover mottled by a finer channel.
  let patches = smoothstep(0.7, 0.82, n.a) * (0.45 + 0.55 * n.b) * amount * 0.5;
  let colour = srgb_to_linear(mix(vec3<f32>(0.42, 0.45, 0.36), vec3<f32>(0.5, 0.4, 0.2), smoothstep(0.55, 0.7, n.g)));
  return vec4<f32>(colour, patches);
}

// Dark water streaks down faces steeper than 60 degrees, straight down
// the fall line: a 1D noise across the face, stretched to 60 m tall runs
// that start and stop. Stronger where the ground is moist. Returns how
// much to darken, 0 to about 0.4.
fn rock_streaks(position: vec3<f32>, ddx_p: vec3<f32>, ddy_p: vec3<f32>, normal: vec3<f32>, moisture: f32) -> f32 {
  let steep = 1.0 - smoothstep(0.42, 0.52, normal.y);

  if (steep <= 0.001) {
    return 0.0;
  }

  let along = normalize(vec2<f32>(-normal.z, normal.x) + vec2<f32>(1e-5, 0.0));
  let uv = vec2<f32>(dot(position.xz, along) / 7.0, position.y / 60.0);
  let duv_x = vec2<f32>(dot(ddx_p.xz, along) / 7.0, ddx_p.y / 60.0);
  let duv_y = vec2<f32>(dot(ddy_p.xz, along) / 7.0, ddy_p.y / 60.0);
  let across = textureSampleGrad(noise_texture, linear_sampler, vec2<f32>(uv.x, 0.37), vec2<f32>(duv_x.x, 0.0), vec2<f32>(duv_y.x, 0.0)).g;
  let runs = textureSampleGrad(noise_texture, linear_sampler, uv * vec2<f32>(0.5, 1.0), duv_x * vec2<f32>(0.5, 1.0), duv_y * vec2<f32>(0.5, 1.0)).r;
  return smoothstep(0.55, 0.8, across) * smoothstep(0.3, 0.6, runs) * steep * saturate(moisture * 1.6 - 0.2) * 0.4;
}
