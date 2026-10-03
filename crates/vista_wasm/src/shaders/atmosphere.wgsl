// Full-screen composite pass: sky, sun, clouds, aerial perspective, and
// ground mist, followed by tone mapping. `common.wgsl` is prepended.
//
// Opaque geometry (terrain, trees, grass) is rendered first into a linear
// HDR target. This pass reads that target and the depth buffer, so it can:
//
// - draw the analytic sky, sun disc, and clouds behind everything;
// - march clouds in front of geometry too (mountains poking into cloud);
// - apply haze and height-based mist with the true distance to every
//   pixel, in one place, instead of each object shader approximating it.
//
// Clouds are raymarched in their own pass (`cloud_main`) at a reduced
// resolution (`CloudsOptions.resolutionScale`) into an `rgba16float` target
// holding in-scattered light and transmittance; the composite upsamples it.
// Clouds are soft, so half resolution is indistinguishable from full at a
// quarter of the cost. The composite also draws rain and snow.
//
// Clouds have two styles. `Painted` shades a single 2D weather layer with
// a cheap sun-offset self-shadow. `Volumetric` raymarches a cloud slab
// through baked 3D Perlin-Worley noise, with a short secondary march
// towards the sun (Beer-powder lighting), a dual-lobe phase function for
// silver linings, and wind drift plus slow billowing evolution.

@group(3) @binding(0) var scene_texture: texture_2d<f32>;
@group(3) @binding(1) var depth_texture: texture_depth_2d;
@group(3) @binding(2) var cloud_texture_low: texture_2d<f32>;
// The finished frame, read by the lens-drop pass only.
@group(3) @binding(3) var lens_source: texture_2d<f32>;
// Raindrops on the lens (see `lens_drops.rs`): xy centre and z radius in
// screen heights, w a bead's fade, or 2 plus a running drop's sideways
// direction mapped to 0 to 1. The bins hold the tile grid's columns and
// rows, then per tile its first list index shifted up 8 bits plus its drop
// count, then the list of drop indices. Read by the lens-drop pass only.
@group(3) @binding(6) var<storage, read> lens_drops: array<vec4<f32>>;
@group(3) @binding(7) var<storage, read> lens_bins: array<u32>;
// The previous frame's clouds, and this frame's quarter-size march, read
// by the cloud pass when reusing clouds.
@group(3) @binding(4) var cloud_history: texture_2d<f32>;
@group(3) @binding(5) var cloud_quarter: texture_2d<f32>;

struct VertexOut {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) ndc: vec2<f32>,
};

@vertex
fn vertex_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
  var positions = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(3.0, -1.0),
    vec2<f32>(-1.0, 3.0)
  );
  let position = positions[vertex_index];

  var out: VertexOut;
  out.clip_position = vec4<f32>(position, 0.0, 1.0);
  out.ndc = position;
  return out;
}

fn view_ray(ndc: vec2<f32>) -> vec3<f32> {
  let tan_half_fov_y = max(frame.camera_forward.w, 0.0001);
  let aspect = max(frame.camera_right.w, 0.0001);
  return normalize(
    frame.camera_forward.xyz
      + frame.camera_right.xyz * ndc.x * tan_half_fov_y * aspect
      + frame.camera_up.xyz * ndc.y * tan_half_fov_y
  );
}

fn linear_distance(depth: f32, ray: vec3<f32>) -> f32 {
  let near = frame.water_deep.w;
  let far = frame.water_current.w;
  let view_depth = far * near / max(far - depth * (far - near), 0.0001);
  return view_depth / max(dot(ray, frame.camera_forward.xyz), 0.0001);
}

// --- Clouds ---------------------------------------------------------------

fn cloud_wind_offset() -> vec3<f32> {
  return vec3<f32>(frame.cloud_motion.x, frame.cloud_motion.z, frame.cloud_motion.y);
}

fn cloud_height_fraction(p: vec3<f32>) -> f32 {
  return (p.y - frame.cloud_params.y) / max(frame.cloud_params.z, 1.0);
}

// Large-scale cloud shape: which parts of the slab hold cloud at all.
// Only this cheap part is used for light marching. It blends three cloud
// types:
//
// - cumulus: flat bases and rounded domes, taller where the weather is
//   denser, narrowing with height so they never rise as columns;
// - stratiform sheets (stratus, nimbostratus): low, flat, and nearly
//   uniform, driven by `clouds3.x`;
// - towering storm clouds (cumulonimbus): widely spaced cores that rise
//   through the whole stretched slab and spread into anvils at the top,
//   driven by `clouds3.y`.
//
// Bases are mostly flat, because clouds form at a condensation level, but
// each heaped cloud's level differs a little (`clouds5.x`, fading out as
// the clouds turn stratiform), and bases curl up gently at their rims. Ragged bases (`clouds3.w`) lift and roughen the
// underside further and scatter loose scraps of cloud (scud) beneath it.
//
// `level` is the base level from `cloud_weather_level`. Returns x the
// shape, y the local base and z the local top, both in shares of the
// ordinary layer's thickness, for `cloud_density` and the underside's
// shading.
fn cloud_shape_at(p: vec3<f32>, weather: f32, level: f32) -> vec3<f32> {
  let h = cloud_height_fraction(p);

  if (h <= 0.0 || h >= 1.0) {
    return vec3<f32>(0.0, 0.0, 1.0);
  }

  let stratiform = frame.clouds3.x;
  let towering = frame.clouds3.y;
  let ragged = frame.clouds3.w;
  let stretch = frame.clouds4.w;
  let wind = cloud_wind_offset();
  // Height within the ordinary cloud layer (towers rise above 1).
  let layer_h = h * stretch;
  // Heaped clouds each condense at their own level; flat sheets keep the
  // flat (or ragged) base of one layer.
  let variation = frame.clouds5.x * (1.0 - stratiform);
  // Each cloud's own base above the slab floor, before its rim curls up.
  let lift = select(0.0, cloud_base_lift(level, 1.0, variation), variation > 0.0001);

  // Below its own base, and the scud that hangs under it, is clear air:
  // skip the rest.
  if (layer_h < lift) {
    return vec3<f32>(0.0, 0.0, 1.0);
  }

  let low = textureSampleLevel(cloud_texture, linear_sampler, (p + wind) / 4200.0, 0.0).r;
  var bottom = 0.0;

  if (ragged > 0.001) {
    let undulation = textureSampleLevel(noise_texture, linear_sampler, (p.xz + wind.xz) / 2600.0, 0.0).a;
    bottom = ragged * 0.14 * undulation;
  }

  let scud_bottom = bottom;

  // Storm cells: Worley cell centres about 6 km apart, warped so their
  // outlines are irregular, or the regional map's storm cells where the
  // weather varies across the map. Inside a cell the cumulus grows much
  // taller, which gives billowing towers that narrow naturally.
  var cell = 0.0;
  var anvil = 0.0;

  if (towering > 0.001) {
    var cells = 0.0;

    if (frame.regional.w > 0.5) {
      // The map's cells are smooth at its 500 m texels; a little noise
      // frays their outlines.
      let storm = regional_weather_at(p.xz).b;
      let fray = textureSampleLevel(noise_texture, linear_sampler, (p.xz + wind.xz) / 5000.0, 0.0).r - 0.5;
      cells = 0.45 + 0.4 * storm + fray * 0.15 * storm;
    } else {
      let warp = textureSampleLevel(noise_texture, linear_sampler, (p.xz + wind.xz) / 14000.0, 0.0).rg - 0.5;
      cells = textureSampleLevel(noise_texture, linear_sampler, (p.xz + wind.xz * 0.5 + warp * 3500.0) / 36000.0 + vec2<f32>(0.31, 0.17), 0.0).b;
    }

    cell = smoothstep(0.55, 0.85, cells) * towering;
    // The anvil: a thin, smoother layer near the top of the slab that
    // spreads well beyond the tower beneath it.
    let band = smoothstep(0.8, 0.86, h) * (1.0 - smoothstep(0.92, 0.97, h));
    let reach = smoothstep(0.45, 0.75, cells) * towering;
    anvil = saturate(remap(mix(low, 0.8, 0.5) * band * reach, 0.35, 0.7, 0.0, 1.0));
  }

  // Cells make clouds taller and a little denser, but their outlines
  // still come from the 3D noise. Forcing full coverage inside a cell
  // would extrude the 2D cell outline into a straight-walled column.
  let local_weather = weather + (1.0 - weather) * cell * 0.3;
  let top = mix(mix(0.3, 1.0, weather), stretch * 0.9, cell);

  // Neighbouring clouds sit tens of metres apart, and each base tilts a
  // little. Sheets have no rims to curl up.
  if (variation > 0.0001) {
    bottom = bottom + cloud_base_lift(level, mix(1.0, low, 1.0 - stratiform), variation);
  }

  let heap = saturate(remap(layer_h, bottom, bottom + 0.07, 0.0, 1.0))
    * saturate(remap(layer_h, top * mix(0.45, 0.65, cell), top, 1.0, 0.0));
  let cumulus = saturate(remap(low * heap, 1.0 - local_weather * 0.9, 1.0, 0.0, 1.0));
  var base = cumulus;
  var local_top = top;

  if (stratiform > 0.001) {
    let sheet_top = mix(0.25, 0.6, weather);
    let flat_profile = saturate(remap(layer_h, bottom, bottom + 0.05, 0.0, 1.0))
      * saturate(remap(layer_h, sheet_top * 0.6, sheet_top, 1.0, 0.0));
    let sheet = saturate(remap(mix(low, 1.0, 0.45) * flat_profile, 1.0 - weather * 0.95, 1.0, 0.0, 1.0));
    // Storm towers push up through the sheet.
    base = mix(cumulus, sheet, stratiform * (1.0 - cell));
    local_top = mix(top, sheet_top, stratiform * (1.0 - cell));
  }

  // Scud hangs as far below each cloud's own base as ragged bases reach.
  if (ragged > 0.001 && scud_bottom > 0.0) {
    let hang = bottom - scud_bottom;
    let scud_band = (1.0 - smoothstep(bottom - scud_bottom * 0.4, bottom, layer_h)) * smoothstep(hang, hang + 0.015, layer_h);
    base = max(base, saturate(remap(low, 0.62, 0.9, 0.0, 1.0)) * scud_band * ragged * 0.7);
  }

  return vec3<f32>(max(base * saturate(local_weather * 1.6), anvil), bottom, max(local_top, bottom + 0.05));
}

fn cloud_shape(p: vec3<f32>, weather: vec2<f32>) -> f32 {
  return cloud_shape_at(p, weather.x, weather.y).x;
}

// How far a low cloud's base sits above the slab floor, in shares of the
// layer's thickness, from the base-level noise `level` (0 to 1), the
// coarse cloud field `low` and the variation. The offset spans
// `variation` either side of its mean, which is raised by `variation` so
// the lowest bases rest on the floor. Towards a cloud's rim, where `low`
// falls, the base curls up by up to 3.5 % of the thickness, so it
// rounds off instead of ending in a sharp, flat edge. A port lives in the
// tests (`cloud_base_lift` in `render/cloud_tests.rs`), with 1 for `low`
// giving the lowest the base can be.
fn cloud_base_lift(level: f32, low: f32, variation: f32) -> f32 {
  return variation + (level - 0.5) * 2.0 * variation + 0.035 * (1.0 - low);
}

// Full density: the shape eroded by fine Worley detail, which turns the
// edges into cauliflower billows at the top and soft wisps at the base.
// `detail` fades the erosion out with distance: far away the march steps
// are longer than the detail noise, which would alias into streaks.
// `span` is the cloud's local base and top (`cloud_shape_at`): the lowest
// 18 % of it is eroded into soft, rounded lumps instead of wisps.
fn cloud_density(p: vec3<f32>, shape: f32, detail_amount: f32, span: vec2<f32>) -> f32 {
  let h = saturate(cloud_height_fraction(p));

  if (detail_amount <= 0.0) {
    return shape * (0.35 + frame.cloud_motion.w * 1.3);
  }

  // Only the two coarser Worley octaves: the finest is smaller than a
  // march step, and without temporal accumulation it reads as hair.
  let detail = textureSampleLevel(cloud_texture, linear_sampler, (p + cloud_wind_offset() * 1.4) / 560.0, 0.0);
  let fbm = detail.g * 0.7 + detail.b * 0.3;
  var erosion = mix(1.0 - fbm, fbm, saturate(h * 4.0));

  if (frame.clouds5.y > 0.001) {
    // The underside's cotton-wool lumps: the coarse Worley octave
    // inverted erodes the creases between cells and leaves their middles,
    // at 0.6 of the tops' strength, so bumps of 30 to 80 m hang from a
    // base that stays mostly flat. Thin fringes keep their wisps.
    let local_h = (cloud_height_fraction(p) * frame.clouds4.w - span.x) / max(span.y - span.x, 0.02);
    let underside = (1.0 - smoothstep(0.1, 0.18, local_h)) * smoothstep(0.15, 0.25, shape) * frame.clouds5.y;
    erosion = mix(erosion, (1.0 - detail.g) * 0.6, underside);
  }

  let density = saturate(remap(shape, erosion * 0.32 * detail_amount, 1.0, 0.0, 1.0));
  return density * (0.35 + frame.cloud_motion.w * 1.3);
}

struct CloudResult {
  scatter: vec3<f32>,
  transmittance: f32,
  distance: f32,
};

// Sky and ground light reaching a point in the cloud: bright from the sky
// dome above, darker at the base, which is lit only by light bouncing off
// the ground and sea.
fn cloud_ambient(h: f32) -> vec3<f32> {
  var sky = sky_radiance(vec3<f32>(0.0, 1.0, 0.0));

  // Under a mid-level veil the light from above is its soft grey, not the
  // blue of the sky beyond it.
  if (alto_on()) {
    let veil = 1.0 - exp(-ALTO_EXTINCTION * (frame.clouds5.z * 0.35 * ALTOCUMULUS_THICKNESS + frame.clouds5.w * 0.85 * ALTOSTRATUS_THICKNESS));
    let grey = vec3<f32>(luminance(sky)) + sun_light() * max(sun_dir().y, 0.0) * 0.06;
    sky = mix(sky, grey, veil);
  }

  let ground = sun_light() * max(sun_dir().y, 0.0) * 0.08;
  // Rain-laden bases (`clouds3.z`) absorb much more of the light that
  // reaches them.
  let laden = 1.0 - frame.clouds3.z * 0.8 * pow(1.0 - h, 1.5);
  return (sky * mix(0.45, 1.25, h) + ground * (1.0 - h)) * laden;
}

// `cloud_ambient` inside one cloud, `local_h` up from its own base (0) to
// its top (1): the cloud above shades its own underside, which reads a
// soft mid-grey even in sunshine, while its sides stay bright.
fn cloud_ambient_within(h: f32, local_h: f32) -> vec3<f32> {
  return cloud_ambient(h) * mix(0.55, 1.0, smoothstep(0.0, 0.25, local_h));
}

// A stable dither for the march start, evenly spread in both directions
// (the R2 low-discrepancy sequence). Random per-frame jitter shows as
// crawling grain, and a dither that varies mostly along one axis shows as
// stripes; this does neither.
fn march_dither(pixel: vec2<f32>) -> f32 {
  return fract(dot(floor(pixel), vec2<f32>(0.7548776662, 0.5698402910)));
}

fn march_clouds(ray: vec3<f32>, max_distance: f32, pixel: vec2<f32>) -> CloudResult {
  var result = CloudResult(vec3<f32>(0.0), 1.0, 0.0);
  let coverage = frame.cloud_params.x;

  if (coverage <= 0.001) {
    return result;
  }

  let camera = frame.camera_position.xyz;
  let base = frame.cloud_params.y;
  let top = base + max(frame.cloud_params.z, 1.0);
  var t0 = 0.0;
  var t1 = 0.0;

  if (camera.y < base) {
    if (ray.y <= 0.004) {
      return result;
    }
    t0 = (base - camera.y) / ray.y;
    t1 = (top - camera.y) / ray.y;
  } else if (camera.y > top) {
    if (ray.y >= -0.004) {
      return result;
    }
    t0 = (top - camera.y) / ray.y;
    t1 = (base - camera.y) / ray.y;
  } else {
    t0 = 0.0;
    if (ray.y > 0.004) {
      t1 = (top - camera.y) / ray.y;
    } else if (ray.y < -0.004) {
      t1 = (base - camera.y) / ray.y;
    } else {
      t1 = 40000.0;
    }
  }

  // The cloud render distance caps how far clouds are marched.
  t1 = min(t1, min(min(max_distance, frame.distances.z), t0 + 40000.0));

  if (t1 <= t0 || t0 > 90000.0) {
    return result;
  }

  let sun = sun_dir();
  let mu = dot(ray, sun);
  // Dual-lobe phase: strong forward scattering (silver linings) plus some
  // back scattering so clouds facing away from the sun are not flat.
  let phase = mix(henyey_greenstein(mu, 0.75), henyey_greenstein(mu, -0.25), 0.3) * 4.0 * PI;
  let sun_colour = sun_light() * frame.cloud_colour.rgb;
  let steps = frame.cloud_params.w;
  result.distance = t0;

  if (steps < 0.5) {
    // Painted: one layer at the middle of the slab.
    let mid = (t0 + t1) * 0.5;
    let p = camera + ray * mid;
    let weather = cloud_weather(p.xz);
    let fine = textureSampleLevel(noise_texture, linear_sampler, (p.xz + frame.cloud_motion.xy * 1.3) / 2600.0, 0.0);
    let density = saturate(remap(weather * (0.55 + fine.a * 0.9), 0.08, 0.75, 0.0, 1.0)) * saturate(frame.cloud_motion.w * 1.6 + 0.2);
    let towards_sun = cloud_weather(p.xz + sun.xz * 900.0);
    let self_shadow = exp(-max(towards_sun - weather * 0.6, 0.0) * 3.0);
    let light = sun_colour * self_shadow * phase * 0.5 + cloud_ambient(0.6) * 0.5;
    let opacity = density * saturate(abs(ray.y) * 6.0);
    result.scatter = light * opacity;
    result.transmittance = 1.0 - opacity;
    result.distance = mid;
    return result;
  }

  // Adaptive march: steps grow with distance (a cloud far away covers
  // few pixels), empty sky is crossed in double steps using only the 2D
  // weather map, and steps shrink inside clouds. With at most four
  // iterations per requested step (most of them cheap empty-sky skips), a
  // 32-step budget samples a nearby cumulus 15 to 30 times instead of once
  // or twice.
  let iterations = i32(steps) * 4;
  let sigma = 0.045;
  var t = t0 + (30.0 + t0 * 0.02) * march_dither(pixel);
  var transmittance = 1.0;
  var scatter = vec3<f32>(0.0);
  var weighted_distance = 0.0;
  var weight_total = 0.0;

  // Multiple scattering (after Wrenninge): each extra order is dimmer,
  // sees less extinction, and is less forward-peaked. This is what makes
  // real cumulus glow white inside instead of looking like grey wool.
  let phase0 = mix(henyey_greenstein(mu, 0.8), henyey_greenstein(mu, -0.2), 0.25) * 4.0 * PI;
  let phase1 = mix(henyey_greenstein(mu, 0.4), henyey_greenstein(mu, -0.1), 0.25) * 4.0 * PI;
  let phase2 = mix(henyey_greenstein(mu, 0.2), 1.0 / (4.0 * PI), 0.5) * 4.0 * PI;

  var fine = false;
  var misses = 0;
  var last_step = 0.0;
  var was_empty = true;
  var entry_samples = 0;

  for (var i = 0; i < iterations; i = i + 1) {
    if (t >= t1) {
      break;
    }

    let base_step = 30.0 + t * 0.02;
    var p = camera + ray * t;
    var sky = cloud_weather_level(p.xz);
    var weather = sky.x;
    var shaped = vec3<f32>(0.0, 0.0, 1.0);

    if (weather > 0.01) {
      shaped = cloud_shape_at(p, weather, sky.y);
    }

    var shape = shaped.x;

    if (shape <= 0.0) {
      // Leave fine mode after a run of empty samples.
      if (fine) {
        misses = misses + 1;
        fine = misses < 6;
      }

      was_empty = true;
      last_step = select(select(base_step, base_step * 2.0, weather <= 0.01), base_step * 0.5, fine);
      t = t + last_step;
      continue;
    }

    if (was_empty && last_step > 0.0) {
      // The ray just entered a cloud somewhere within the last step.
      // Bisect to find the edge: otherwise every pixel's edge lands at a
      // different point within a step of tens of metres, and silhouettes
      // turn hairy.
      var outside = t - last_step;
      var inside = t;

      for (var k = 0; k < 4; k = k + 1) {
        let mid = (outside + inside) * 0.5;
        let q = camera + ray * mid;
        let w = cloud_weather_level(q.xz);
        let hit = w.x > 0.01 && cloud_shape(q, w) > 0.0;
        outside = select(mid, outside, hit);
        inside = select(inside, mid, hit);
      }

      t = inside;
      p = camera + ray * t;
      sky = cloud_weather_level(p.xz);
      weather = sky.x;
      shaped = cloud_shape_at(p, weather, sky.y);
      shape = max(shaped.x, 0.0001);
    }

    if (was_empty) {
      entry_samples = 0;
    }

    was_empty = false;
    fine = true;
    misses = 0;
    let density = cloud_density(p, shape, 1.0 - smoothstep(2500.0, 9000.0, t), shaped.yz);
    // Short steps just inside an edge: a ray grazing the thin top of a
    // cloud otherwise catches it at one pixel and misses it at the next,
    // which reads as hair along the silhouette.
    let step_length = base_step * select(0.5, 0.2, entry_samples < 4);
    entry_samples = entry_samples + 1;
    last_step = step_length;
    t = t + step_length;

    if (density <= 0.002) {
      continue;
    }

    // Light march towards the sun through the cheap shape, with steps
    // doubling in length to reach the far side of tall towers.
    var optical = 0.0;
    var light_step = 40.0;
    var light_t = light_step * 0.5;
    // Under a flat overcast sheet the light arriving is diffuse and the
    // ambient term dominates, so three light samples do the work of five.
    // The count is the same for the whole frame, so no pixel waits on
    // another.
    let light_samples = select(5, 3, frame.clouds3.x > 0.6);

    for (var j = 0; j < light_samples; j = j + 1) {
      let lp = p + sun * light_t;
      let lw = cloud_weather_level(lp.xz);
      optical = optical + cloud_shape(lp, lw) * (0.35 + frame.cloud_motion.w * 1.3) * light_step;
      light_step = light_step * 2.0;
      light_t = light_t + light_step * 0.75;
    }

    let depth = optical * sigma;
    let direct = exp(-depth) * phase0 + exp(-depth * 0.35) * phase1 * 0.45 + exp(-depth * 0.12) * phase2 * 0.2;
    // Powder: edges facing the sun are thin and scatter little back.
    let powder = mix(1.0, 1.0 - exp(-density * 6.0), 0.4 * (1.0 - saturate(mu)));
    let layer_h = cloud_height_fraction(p) * frame.clouds4.w;
    let h = saturate(layer_h);
    let local_h = (layer_h - shaped.y) / max(shaped.z - shaped.y, 0.02);
    let laden = 1.0 - frame.clouds3.z * 0.5 * (1.0 - h);
    // The mid-level layer above shades the sunlight reaching the cloud.
    let veil = alto_shadow(p);
    var light = sun_colour * direct * powder * laden * veil
      + cloud_ambient_within(h, local_h) * frame.cloud_colour.rgb * exp(-density * 0.5);

    // Lightning lights the cloud around the strike from inside.
    if (frame.weather2.x > 0.001) {
      let strike = distance(p.xz, frame.clouds4.yz);
      light = light + vec3<f32>(0.85, 0.88, 1.0) * frame.weather2.x * 40.0 * exp(-strike / 1200.0) * (1.0 - h * 0.4);
    }

    let step_transmittance = exp(-density * sigma * step_length);
    let absorbed = transmittance * (1.0 - step_transmittance);
    scatter = scatter + light * absorbed;
    weighted_distance = weighted_distance + t * absorbed;
    weight_total = weight_total + absorbed;
    transmittance = transmittance * step_transmittance;

    if (transmittance < 0.01) {
      break;
    }
  }

  result.scatter = scatter;
  result.transmittance = transmittance;
  result.distance = select(t0, weighted_distance / max(weight_total, 0.0001), weight_total > 0.0001);
  return result;
}

// Thin, high cirrus drawn as a single layer: wind-stretched streaks with
// strong forward scattering, so they glow around the sun. Returns light
// (rgb) and opacity (a).
fn cirrus(ray: vec3<f32>) -> vec4<f32> {
  let amount = frame.clouds2.x;

  if (amount <= 0.001 || ray.y <= 0.01) {
    return vec4<f32>(0.0);
  }

  let camera = frame.camera_position.xyz;
  let t = (frame.clouds2.y - camera.y) / ray.y;

  if (t <= 0.0) {
    return vec4<f32>(0.0);
  }

  // Cirrus drifts with its own speed (`cirrusSpeed`), not the low clouds'.
  let xz = camera.xz + ray.xz * t + frame.weather3.xy;
  // Rotate into the wind frame and stretch along the wind.
  let along = frame.clouds2.zw;
  let across = vec2<f32>(-along.y, along.x);
  let local = vec2<f32>(dot(xz, along) / 2.5, dot(xz, across)) / 14000.0;
  let broad = textureSampleLevel(noise_texture, linear_sampler, local * 0.45 + vec2<f32>(0.13, 0.71), 0.0).r;
  let streaks = textureSampleLevel(noise_texture, linear_sampler, local * vec2<f32>(0.7, 1.4), 0.0).g;
  let fine = textureSampleLevel(noise_texture, linear_sampler, local * vec2<f32>(1.2, 2.6) + vec2<f32>(0.4, 0.2), 0.0).a;
  let field = broad * 0.5 + streaks * 0.35 + fine * 0.15;
  let threshold = 0.72 - amount * 0.4;
  let density = smoothstep(threshold, threshold + 0.3, field);
  // Fade out towards the horizon, where the layer would alias.
  let opacity = density * 0.55 * smoothstep(0.01, 0.15, ray.y);
  let mu = dot(ray, sun_dir());
  let phase = mix(henyey_greenstein(mu, 0.7), 1.0 / (4.0 * PI), 0.4) * 4.0 * PI;
  let light = (sun_light() * phase * 0.9 + sky_radiance(vec3<f32>(0.0, 1.0, 0.0)) * 0.9) * frame.cloud_colour.rgb;
  return vec4<f32>(light * opacity, opacity);
}

// The mid-level layer's density where `ray` meets its plane: x the
// distance there (0 when it does not), y the altocumulus and z the
// altostratus density, w the optical depth along the ray. The cloudlets
// fade to their mean where they would span fewer than about three pixels
// of the cloud image, and the ripples likewise, so lone specks never
// twinkle as the layer drifts and it never shimmers towards the horizon.
fn alto_along(ray: vec3<f32>) -> vec4<f32> {
  let camera = frame.camera_position.xyz;

  if (!alto_on() || abs(ray.y) < 0.01) {
    return vec4<f32>(0.0);
  }

  let t = (frame.alto.z - camera.y) / ray.y;

  if (t <= 0.0) {
    return vec4<f32>(0.0);
  }

  let xz = camera.xz + ray.xz * t;
  let coarse = alto_coarse(xz);
  var cumulus = 0.0;

  if (coarse.x > 0.001) {
    let q = xz + frame.alto.xy;
    // Metres of the layer one pixel of the cloud image spans.
    let footprint = t * 2.0 * frame.camera_forward.w / max(frame.temporal.w, 1.0) / max(abs(ray.y), 0.05);
    // A small warp keeps the cells from repeating every tile.
    let warp = textureSampleLevel(noise_texture, linear_sampler, q / 7000.0 + vec2<f32>(0.71, 0.07), 0.0).rg - 0.5;
    let level = log2(max(footprint / (ALTO_CELL_TILE / 512.0), 1.0));
    let cells = textureSampleLevel(noise_texture, linear_sampler, (q + warp * 500.0) / ALTO_CELL_TILE, level).b;
    // The warp also bends the rows, so they wander and break like real
    // billows instead of running as ruled lines.
    let ripple = mix(alto_ripple(q, coarse.z + warp.x * 5.0), 0.5, smoothstep(100.0, 300.0, footprint));
    // Cloudlets where the cells rise above a threshold that falls with
    // the amount; the ripples' troughs thin the rows apart.
    let threshold = 0.95 - 0.6 * coarse.x;
    let rise = mix(0.6, 1.0, ripple);
    // A cloudlet's edge, in the cells' units. Where only a cell's very
    // middle would cross the threshold, a cloudlet is drawn at the
    // smallest size instead, fading out below it, so a sparse layer
    // leaves no lone specks.
    let size = 1.0 - threshold / rise;
    let edge = 1.0 - max(size, ALTO_SMALLEST);
    let presence = smoothstep(0.5 * ALTO_SMALLEST, ALTO_SMALLEST, size);
    // A soft edge, and more cloud towards each cell's middle, so the
    // cloudlets are rounded puffs rather than flat cut-outs.
    let cloudlets = presence * smoothstep(edge, edge + 0.35 / rise, cells) * (0.55 + 0.45 * smoothstep(edge, 1.0 / rise, cells));
    cumulus = mix(cloudlets, coarse.x * mix(0.25, 0.45, ripple), smoothstep(30.0, 90.0, footprint));
  }

  let tau = ALTO_EXTINCTION * (cumulus * ALTOCUMULUS_THICKNESS + coarse.y * ALTOSTRATUS_THICKNESS) / max(abs(ray.y), 0.05);
  return vec4<f32>(t, cumulus, coarse.y, tau);
}

// The mid-level layer along `ray`, up to `max_distance`: light and
// transmittance, lit by the sun through a cheap self-shadow (one lookup
// towards the sun), a forward and an even phase lobe, a powder term for
// silver edges near the sun, and the sky, dimmer under a thick layer.
fn alto_layer(ray: vec3<f32>, max_distance: f32) -> CloudResult {
  var result = CloudResult(vec3<f32>(0.0), 1.0, 0.0);
  let along = alto_along(ray);

  if (along.x <= 0.0 || along.x >= max_distance || along.w <= 0.0001) {
    return result;
  }

  // Faded towards the horizon, where the layer would alias, into the haze.
  let opacity = (1.0 - exp(-along.w)) * smoothstep(0.01, 0.12, abs(ray.y));
  let sun = sun_dir();
  let mu = dot(ray, sun);
  let sun_y = max(sun.y, 0.05);
  let thickness = mix(ALTOCUMULUS_THICKNESS, ALTOSTRATUS_THICKNESS, along.z / max(along.y + along.z, 0.0001));
  let xz = frame.camera_position.xz + ray.xz * along.x;
  // Light reaching the middle of the layer crosses about half of it.
  let towards_sun = alto_shadow_depth(xz + sun.xz / sun_y * thickness * 0.5) / sun_y;
  let phase = mix(henyey_greenstein(mu, 0.6), 1.0 / (4.0 * PI), 0.5) * 4.0 * PI;
  // Light scattered once, and light scattered many times: dimmer, deeper,
  // even in every direction and as strong as the sunlight falling on the
  // layer. A thick veil is a soft grey, not a glare.
  let direct = exp(-towards_sun * 0.7) * phase * 0.6 + exp(-towards_sun * 0.3) * 0.3 * sun_y;
  let powder = mix(1.0, 1.0 - exp(-along.w * 2.0), 0.5 * (1.0 - saturate(mu)));
  let straight = along.w * abs(ray.y);
  let below = frame.camera_position.y < frame.alto.z;
  let sky = sky_radiance(vec3<f32>(0.0, 1.0, 0.0)) * select(1.1, mix(0.45, 1.0, exp(-straight * 1.2)), below);
  let light = (sun_light() * direct * powder * 0.9 + sky) * frame.cloud_colour.rgb;
  result.scatter = light * opacity;
  result.transmittance = 1.0 - opacity;
  result.distance = along.x;
  return result;
}

fn sun_disc(ray: vec3<f32>) -> vec3<f32> {
  let cos_angle = dot(ray, sun_dir());
  let disc = smoothstep(0.99995, 0.999985, cos_angle);
  let glow = pow(saturate(cos_angle), 2400.0) * 0.4;
  return sun_transmittance(sun_dir().y) * frame.sun_direction.w * (disc * 900.0 + glow * 6.0);
}

// Distance along `ray` at which it enters the cloud slab, or a huge value
// when it never does.
fn cloud_entry_distance(ray: vec3<f32>) -> f32 {
  let camera_y = frame.camera_position.y;
  let base = frame.cloud_params.y;
  let top = base + max(frame.cloud_params.z, 1.0);

  if (camera_y >= base && camera_y <= top) {
    return 0.0;
  }

  if (camera_y < base) {
    return select(1.0e9, (base - camera_y) / ray.y, ray.y > 0.004);
  }

  return select(1.0e9, (top - camera_y) / ray.y, ray.y < -0.004);
}

// Distance along `ray` at which it crosses the mid-level layer, or a huge
// value when it never does.
fn alto_entry_distance(ray: vec3<f32>) -> f32 {
  let t = (frame.alto.z - frame.camera_position.y) / ray.y;
  return select(1.0e9, t, alto_on() && abs(ray.y) >= 0.01 && t > 0.0);
}

// Full-resolution pixel covered by a fragment of the reduced cloud target.
fn full_pixel(ndc: vec2<f32>) -> vec2<i32> {
  let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
  return vec2<i32>(clamp(uv * frame.viewport.xy, vec2<f32>(0.0), frame.viewport.xy - 1.0));
}

// Rain curtains are only marched from this distance outwards.
const SHAFT_START: f32 = 1200.0;

// Curtains of rain (or snow) hanging below raining clouds, seen from a
// distance. A short march below the cloud base: curtains sit under
// clouds, break up into shafts, and slant downwind as they fall. Returns
// in-scattered light (rgb), already faded into the haze, and
// transmittance (a). Costs nothing when `clouds4.x` is zero.
fn rain_shafts(ray: vec3<f32>, max_distance: f32) -> vec4<f32> {
  let amount = frame.clouds4.x;

  if (amount <= 0.001) {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
  }

  let camera = frame.camera_position.xyz;
  let base = frame.cloud_params.y;
  let t_end = min(min(max_distance, 16000.0), frame.distances.z);
  // Shafts change slowly across the ground, so fixed sample positions
  // give a smooth result; any per-pixel offset shows as grain.
  let dither = 0.5;
  let snowy = select(0.0, 1.0, frame.weather.y > frame.weather.x);
  // Shafts take the light of the cloud base above them: dark grey under
  // rain-laden cloud, paler for snow.
  let grey = cloud_ambient(0.0) * frame.cloud_colour.rgb * mix(0.55, 1.0, snowy) + sun_light() * 0.02;
  let haze_distance = max(frame.atmosphere.z * 1.4, 1.0);
  // The same for every sample along the ray, so look it up once.
  let far_light = sky_radiance(ray);
  var transmittance = 1.0;
  var scatter = vec3<f32>(0.0);

  for (var i = 0; i < 20; i = i + 1) {
    let u = (f32(i) + dither) / 20.0;
    // Curtains are seen from a distance. Close by, and overhead, you are
    // inside the rain, which the falling streaks already show; a curtain
    // there would hang over the camera as a pulsing disc. So the march
    // starts well away from the camera.
    let t = SHAFT_START + (t_end - SHAFT_START) * u * u;
    let dt = (t_end - SHAFT_START) * 2.0 * u / 20.0;
    let p = camera + ray * t;
    let fall = base - p.y;

    if (fall <= 0.0 || t >= t_end) {
      continue;
    }

    let xz = p.xz - frame.clouds2.zw * fall * 0.25;
    let above = cloud_weather(xz);
    let curtains = textureSampleLevel(noise_texture, linear_sampler, (xz + frame.cloud_motion.xy) / 7000.0, 0.0).r;
    let streaks = textureSampleLevel(noise_texture, linear_sampler, xz / 1800.0, 0.0).g;
    var falling = 1.0;

    // Rain only falls where the regional map has it.
    if (frame.regional.w > 0.5) {
      falling = saturate(regional_weather_at(xz).g * 1.5);
    }

    let density = smoothstep(0.35, 0.7, above) * smoothstep(0.45, 0.75, curtains * 0.75 + streaks * 0.25)
      * saturate(fall / 200.0) * amount * falling;
    let step_transmittance = exp(-density * 0.0006 * dt);
    let haze = exp(-t / haze_distance);
    let light = mix(far_light, grey, haze);
    scatter = scatter + transmittance * (1.0 - step_transmittance) * light;
    transmittance = transmittance * step_transmittance;
  }

  return vec4<f32>(scatter, transmittance);
}

// The previous frame's clouds in direction `ray`, or a negative alpha when
// that direction was off screen. Clouds are kilometres away, so they are
// reprojected by direction: the point 20 km along the ray, as the previous
// camera saw it.
fn previous_clouds(ray: vec3<f32>) -> vec4<f32> {
  let point = frame.camera_position.xyz + ray * 20000.0;
  let clip = frame.previous_view_proj * vec4<f32>(point, 1.0);

  if (clip.w <= 0.0) {
    return vec4<f32>(-1.0);
  }

  let ndc = clip.xy / clip.w;

  if (any(abs(ndc) > vec2<f32>(1.0))) {
    return vec4<f32>(-1.0);
  }

  let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
  return textureSampleLevel(cloud_history, clamp_sampler, uv, 0.0);
}

// Clouds (and rain curtains) along `ray`, stopping at geometry
// `depth`. `pixel` seeds the march's stable dither. Output: rgb =
// in-scattered light already faded into the haze, a = transmittance.
fn clouds_along(ray: vec3<f32>, depth: f32, pixel: vec2<f32>) -> vec4<f32> {
  let is_sky = depth >= 0.999999;
  let max_distance = select(linear_distance(depth, ray), 1.0e9, is_sky);
  let clouds = march_clouds(ray, max_distance, pixel);
  let shafts = rain_shafts(ray, max_distance);
  let alto = alto_layer(ray, max_distance);

  if (clouds.transmittance >= 0.999 && shafts.a >= 0.999 && alto.transmittance >= 0.999) {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
  }

  // Distant clouds fade into the haze near the horizon, and out entirely
  // before the cloud render distance.
  let haze_distance = max(frame.atmosphere.z * 1.4, 1.0);
  let haze = exp(-clouds.distance / haze_distance);
  let sky = sky_radiance(ray);
  let limit = frame.distances.z;
  let keep = 1.0 - fade_in_before(limit, frame.fades.y, clouds.distance);
  let transmittance = mix(1.0, clouds.transmittance, keep);
  var scatter = mix(sky * (1.0 - transmittance), clouds.scatter * keep, haze);
  var through = transmittance;

  // The mid-level layer, hazed the same way, lies behind the low clouds
  // from below and in front of them from above. Like cirrus, it ignores
  // the cloud render distance: it costs the same at any distance.
  if (alto.transmittance < 0.999) {
    let alto_scatter = mix(sky * (1.0 - alto.transmittance), alto.scatter, exp(-alto.distance / haze_distance));

    if (frame.camera_position.y < frame.alto.z) {
      scatter = scatter + through * alto_scatter;
    } else {
      scatter = alto_scatter + alto.transmittance * scatter;
    }

    through = through * alto.transmittance;
  }

  // Rain shafts hang below the clouds, so they sit in front of them.
  return vec4<f32>(shafts.rgb + shafts.a * scatter, shafts.a * through);
}

// Which pixel of each 2 x 2 block of the cloud image is marched this frame
// while clouds are reused.
fn reuse_offset() -> vec2<u32> {
  let phase = u32(frame.temporal.y);
  return vec2<u32>(phase % 2u, phase / 2u);
}

// Reused clouds, part 1: a quarter-size pass that marches one sky pixel of
// every 2 x 2 block of the cloud image. Every pixel here does the same
// work, so none waits on a neighbour; marching one pixel in four inside the
// full-size pass would not help, because GPUs shade pixels in groups and a
// group costs as much as its slowest pixel.
@fragment
fn cloud_quarter_main(in: VertexOut) -> @location(0) vec4<f32> {
  let size = frame.temporal.zw;
  let pixel = floor(in.clip_position.xy) * 2.0 + vec2<f32>(reuse_offset()) + 0.5;
  let ndc = vec2<f32>(pixel.x / size.x * 2.0 - 1.0, 1.0 - pixel.y / size.y * 2.0);
  let depth = textureLoad(depth_texture, full_pixel(ndc), 0);

  // Pixels with terrain in front are marched by the full-size pass.
  if (depth < 0.999999) {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
  }

  return clouds_along(view_ray(ndc), depth, pixel);
}

// Reduced-resolution cloud pass. Output: rgb = in-scattered light already
// faded into the haze, a = transmittance.
@fragment
fn cloud_main(in: VertexOut) -> @location(0) vec4<f32> {
  let ray = view_ray(in.ndc);
  let depth = textureLoad(depth_texture, full_pixel(in.ndc), 0);

  // Reused clouds, part 2. Distant clouds change little from one frame to
  // the next, like the far layers of a parallax scene: each sky pixel takes
  // this frame's quarter-size march when it is its turn, and otherwise the
  // previous frame's clouds, reprojected. Pixels with terrain in front are
  // cheap and always marched here, so silhouettes never smear.
  if (frame.temporal.x > 0.5 && depth >= 0.999999) {
    let pixel = vec2<u32>(in.clip_position.xy);
    let fresh = textureLoad(cloud_quarter, vec2<i32>(pixel / 2u), 0);

    if (all(pixel % 2u == reuse_offset())) {
      return fresh;
    }

    let reused = previous_clouds(ray);
    return select(fresh, reused, reused.a >= 0.0);
  }

  return clouds_along(ray, depth, in.clip_position.xy);
}

@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
  let pixel = vec2<i32>(in.clip_position.xy);
  let depth = textureLoad(depth_texture, pixel, 0);
  let ray = view_ray(in.ndc);
  let uv = in.clip_position.xy * frame.viewport.zw;
  let clouds_on = frame.cloud_params.x > 0.001 || alto_on();
  var colour: vec3<f32>;
  var distance = 1.0e9;

  if (depth >= 0.999999) {
    // Background: sky, sun, and clouds.
    var sky = sky_radiance(ray);

    if (ray.y < 0.0) {
      // Below the horizon with no geometry (water disabled, or beyond the
      // terrain): darken towards a hazy ground colour.
      sky = mix(sky, sky * 0.35, saturate(-ray.y * 3.0));

      // With a render distance, the ground beyond it is distance fog.
      if (frame.distances.x < 1.0e8) {
        sky = mix(sky, render_distance_colour(ray), saturate(-ray.y * 20.0));
      }
    }

    // The disc is hundreds of times brighter than the sky, so even 1 %
    // transmittance would leave it glaring through thick cloud. Hide it
    // smoothly as cloud thickens, and under an overcast sky.
    let disc = sun_disc(ray) * step(0.0, ray.y) * (1.0 - frame.weather2.y);
    colour = sky;

    if (clouds_on) {
      let high = cirrus(ray);
      let clouds = textureSampleLevel(cloud_texture_low, clamp_sampler, uv, 0.0);
      let through = (1.0 - high.a) * clouds.a;
      var veil = 1.0;
      var corona = vec3<f32>(0.0);
      let cos_sun = dot(ray, sun_dir());

      // Within about 3 degrees of the sun, a veil of altostratus dims the
      // disc to a watery one without hiding it, and scatters a soft
      // corona round it, strongest where the veil is half transparent.
      // Each pixel takes the veil along its own ray, at full resolution,
      // so its fibres and gaps brighten and darken the disc and the glow
      // as it drifts: one lookup, for these few thousand pixels only.
      // Altocumulus cloudlets in front of the sun are in the cloud image.
      let rise = frame.alto.z - frame.camera_position.y;

      if (alto_on() && cos_sun > 0.9986 && rise > 0.0 && ray.y > 0.01) {
        let xz = frame.camera_position.xz + ray.xz * (rise / ray.y);
        let depth = ALTO_EXTINCTION * alto_coarse(xz).y * ALTOSTRATUS_THICKNESS / max(ray.y, 0.05);
        veil = 1.0 - (1.0 - exp(-depth)) * smoothstep(0.01, 0.12, ray.y);
        let glow = saturate((cos_sun - 0.9986) / 0.0014);
        corona = sun_light() * frame.cloud_colour.rgb * (1.0 - veil) * veil * glow * glow * 1.6;
      }

      // Only the thick low clouds hide the disc; the veil just dims it.
      let low = min(through / max(veil, 0.0001), 1.0);
      colour = ((colour * (1.0 - high.a) + high.rgb) * clouds.a + clouds.rgb)
        + disc * veil * low * smoothstep(0.05, 0.7, low) + corona * low;
    } else {
      colour = colour + disc;
    }

    let mist = atmospheric_fog(ray, 12000.0, in.clip_position.xy, false);
    colour = colour * mist.transmittance + mist.inscatter;
  } else {
    distance = linear_distance(depth, ray);

    if (beyond_render_distance(distance)) {
      // Past the render distance: only the distance fog is visible.
      colour = render_distance_colour(ray);
    } else {
      let scene = textureLoad(scene_texture, pixel, 0).rgb;
      let fog = atmospheric_fog(ray, distance, in.clip_position.xy, true);
      colour = scene * fog.transmittance + fog.inscatter;

      // Only geometry that reaches into the cloud layer can have clouds in
      // front of it; skipping the rest avoids upsampling halos on low ground.
      if (clouds_on && (cloud_entry_distance(ray) < distance || frame.clouds4.x > 0.001 || alto_entry_distance(ray) < distance)) {
        let clouds = textureSampleLevel(cloud_texture_low, clamp_sampler, uv, 0.0);
        colour = colour * clouds.a + clouds.rgb;
      }

      colour = mix(colour, render_distance_colour(ray), render_distance_fog(distance));
    }
  }

  let falling = precipitation(ray, distance);
  colour = mix(colour, falling.rgb, falling.a);
  return vec4<f32>(finish_colour(colour), 1.0);
}

// Half-resolution copy of the opaque scene for water to reflect: HDR
// colour in rgb, linear view depth in a (1e9 for the sky).
@fragment
fn scene_copy_main(in: VertexOut) -> @location(0) vec4<f32> {
  let pixel = full_pixel(in.ndc);
  let depth = textureLoad(depth_texture, pixel, 0);
  let near = frame.water_deep.w;
  let far = frame.water_current.w;
  let view_depth = select(far * near / max(far - depth * (far - near), 0.0001), 1.0e9, depth >= 0.999999);
  return vec4<f32>(textureLoad(scene_texture, pixel, 0).rgb, view_depth);
}

// The final pass, run when the scene was rendered below the canvas
// resolution or raindrops land on the lens. It upscales the finished frame
// to the canvas, sharpening what was upscaled with a contrast-adaptive
// filter that cannot overshoot its neighbours (so no halos), and refracts
// the frame through raindrops on the lens: small beads that sit still and
// evaporate, and larger drops that run down the screen, each showing the
// scene behind it flipped and magnified, with a darker rim and a glint.
// The drops are simulated on the CPU and binned into screen tiles; each
// pixel tests only its own tile's drops.
@fragment
fn present_main(in: VertexOut) -> @location(0) vec4<f32> {
  let size = frame.output.xy;
  let pixel = in.clip_position.xy;
  var bend = vec2<f32>(0.0);
  var rim = 0.0;
  var glint = 0.0;

  // Uniform: 1 only while drops are on the lens.
  if (frame.weather3.z > 0.5) {
    // Screen heights, so drops stay round on any aspect ratio.
    let screen = pixel / size.y;
    let columns = lens_bins[0];
    let rows = lens_bins[1];
    let column = min(u32(screen.x * size.y / size.x * f32(columns)), columns - 1u);
    let row = min(u32(screen.y * f32(rows)), rows - 1u);
    let entry = lens_bins[2u + row * columns + column];
    let first = 2u + columns * rows + (entry >> 8u);

    // Every drop that reaches this tile is binned into it, so each drop is
    // drawn whole, never cut at a tile's edge.
    for (var k = 0u; k < (entry & 255u); k = k + 1u) {
      let drop = lens_drops[lens_bins[first + k]];
      let radius = max(drop.z, 0.00001);
      var d = screen - drop.xy;
      var fade = drop.w;

      if (drop.w >= 2.0) {
        // Running drops are longer along their path: squeeze the offset
        // along the direction of travel.
        let side = (drop.w - 2.0) / 0.999 * 2.0 - 1.0;
        let path = vec2<f32>(side, sqrt(max(1.0 - side * side, 0.0)));
        d = d - path * dot(d, path) * 0.2;
        fade = 1.0;
      }

      let r = length(d) / radius;
      let inside = (1.0 - smoothstep(0.85, 1.0, r)) * fade;
      let normal = d / radius;
      // Offset in screen heights, converted to pixels below: the drop is a
      // lens that shows the scene behind it flipped and magnified.
      bend = bend - normal * radius * 1.6 * inside;
      rim = max(rim, smoothstep(0.55, 1.0, r) * inside);
      glint = max(glint, (1.0 - smoothstep(0.0, 0.25, length(normal - vec2<f32>(-0.35, -0.4)))) * inside);
    }
  }

  let uv = clamp((pixel + bend * size.y) / size, vec2<f32>(0.0), vec2<f32>(1.0));
  var colour = textureSampleLevel(lens_source, linear_sampler, uv, 0.0).rgb;
  let scale = frame.output.z;

  if (scale < 0.999) {
    let texel = frame.viewport.zw;
    let north = textureSampleLevel(lens_source, linear_sampler, uv - vec2<f32>(0.0, texel.y), 0.0).rgb;
    let south = textureSampleLevel(lens_source, linear_sampler, uv + vec2<f32>(0.0, texel.y), 0.0).rgb;
    let west = textureSampleLevel(lens_source, linear_sampler, uv - vec2<f32>(texel.x, 0.0), 0.0).rgb;
    let east = textureSampleLevel(lens_source, linear_sampler, uv + vec2<f32>(texel.x, 0.0), 0.0).rgb;
    let low = min(min(min(north, south), min(west, east)), colour);
    let high = max(max(max(north, south), max(west, east)), colour);
    // Sharpen more the further the frame was upscaled.
    let amount = saturate((1.0 - scale) * 2.0) * 0.6;
    let sharpened = colour + (colour * 4.0 - north - south - west - east) * amount * 0.25;
    colour = clamp(sharpened, low, high);
  }

  colour = colour * (1.0 - rim * 0.35) + vec3<f32>(glint * 0.35);
  return vec4<f32>(colour, 1.0);
}
