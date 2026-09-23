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
// Clouds have two styles. `Painted` shades a single 2D weather layer with
// a cheap sun-offset self-shadow. `Volumetric` raymarches a cloud slab
// through baked 3D Perlin-Worley noise, with a short secondary march
// towards the sun (Beer-powder lighting), a dual-lobe phase function for
// silver linings, and wind drift plus slow billowing evolution.

@group(2) @binding(0) var scene_texture: texture_2d<f32>;
@group(2) @binding(1) var depth_texture: texture_depth_2d;

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

fn cloud_shape(p: vec3<f32>, weather: f32) -> f32 {
  let base = frame.cloud_params.y;
  let thickness = max(frame.cloud_params.z, 1.0);
  let h = (p.y - base) / thickness;

  if (h <= 0.0 || h >= 1.0) {
    return 0.0;
  }

  // Denser weather grows taller clouds; bases are flat, tops rounded.
  let top = mix(0.3, 1.0, weather);
  let profile = smoothstep(0.0, 0.08, h) * (1.0 - smoothstep(top * 0.55, top, h));
  let shape = textureSampleLevel(cloud_texture, linear_sampler, (p + cloud_wind_offset()) / 5600.0, 0.0).r;
  return saturate(remap(shape * profile, 1.0 - weather * 0.9, 1.0, 0.0, 1.0));
}

fn cloud_density(p: vec3<f32>, weather: f32) -> f32 {
  var density = cloud_shape(p, weather);

  if (density <= 0.0) {
    return 0.0;
  }

  // Erode the edges with higher-frequency Worley detail: wispy at the
  // base, billowing towards the top.
  let base = frame.cloud_params.y;
  let h = saturate((p.y - base) / max(frame.cloud_params.z, 1.0));
  let detail = textureSampleLevel(cloud_texture, linear_sampler, (p + cloud_wind_offset() * 1.35) / 1250.0, 0.0);
  let detail_fbm = detail.g * 0.625 + detail.b * 0.25 + detail.a * 0.125;
  let erosion = mix(detail_fbm, 1.0 - detail_fbm, saturate(h * 3.0));
  density = saturate(remap(density, erosion * 0.38, 1.0, 0.0, 1.0));
  return density * (0.35 + frame.cloud_motion.w * 1.3);
}

struct CloudResult {
  scatter: vec3<f32>,
  transmittance: f32,
  distance: f32,
};

fn cloud_lighting_ambient(h: f32) -> vec3<f32> {
  let sky = sky_radiance(vec3<f32>(0.0, 1.0, 0.0));
  return sky * mix(0.55, 1.4, h) + sun_light() * 0.06;
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

  t1 = min(t1, min(max_distance, t0 + 25000.0));

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
    let light = sun_colour * self_shadow * phase * 0.5 + cloud_lighting_ambient(0.6) * 0.5;
    let opacity = density * saturate(abs(ray.y) * 6.0);
    result.scatter = light * opacity;
    result.transmittance = 1.0 - opacity;
    result.distance = mid;
    return result;
  }

  let step_count = i32(steps);
  // White-noise jitter: a structured dither lines up into visible hatching
  // on long glancing rays, while grain is far less noticeable.
  let jitter = hash12(pixel * 1.37 + vec2<f32>(fract(time_seconds() * 0.37) * 97.0, 11.0));
  let span = t1 - t0;
  let sigma = 0.045;
  let light_step = max(frame.cloud_params.z, 1.0) * 0.12;
  var transmittance = 1.0;
  var scatter = vec3<f32>(0.0);
  var weighted_distance = 0.0;
  var weight_total = 0.0;

  for (var i = 0; i < step_count; i = i + 1) {
    // Steps grow with distance: fine detail close by, coverage far away.
    let u0 = f32(i) / f32(step_count);
    let u1 = f32(i + 1) / f32(step_count);
    let t = t0 + span * pow((f32(i) + jitter) / f32(step_count), 1.6);
    let step_length = span * (pow(u1, 1.6) - pow(u0, 1.6));
    let p = camera + ray * t;
    let weather = cloud_weather(p.xz);

    if (weather <= 0.01) {
      continue;
    }

    let density = cloud_density(p, weather);

    if (density <= 0.001) {
      continue;
    }

    // Secondary march towards the sun for self-shadowing.
    var optical = 0.0;

    for (var j = 0; j < 4; j = j + 1) {
      let lp = p + sun * light_step * (f32(j) + 0.5) * (1.0 + f32(j) * 0.6);
      optical = optical + cloud_shape(lp, cloud_weather(lp.xz)) * light_step * (1.0 + f32(j) * 0.6);
    }

    let beer = exp(-optical * sigma * 0.6);
    let powder = 1.0 - exp(-optical * sigma * 1.2 - density * 0.8);
    let h = saturate((p.y - frame.cloud_params.y) / max(frame.cloud_params.z, 1.0));
    let light = sun_colour * beer * mix(1.0, powder, 0.55) * phase + cloud_lighting_ambient(h) * (0.35 + 0.65 * h);
    let extinction = density * sigma;
    let step_transmittance = exp(-extinction * step_length);
    scatter = scatter + transmittance * light * (1.0 - step_transmittance);
    weighted_distance = weighted_distance + t * transmittance * (1.0 - step_transmittance);
    weight_total = weight_total + transmittance * (1.0 - step_transmittance);
    transmittance = transmittance * step_transmittance;

    if (transmittance < 0.02) {
      break;
    }
  }

  result.scatter = scatter;
  result.transmittance = transmittance;
  result.distance = select(t0, weighted_distance / max(weight_total, 0.0001), weight_total > 0.0001);
  return result;
}

fn sun_disc(ray: vec3<f32>) -> vec3<f32> {
  let cos_angle = dot(ray, sun_dir());
  let disc = smoothstep(0.99995, 0.999985, cos_angle);
  let glow = pow(saturate(cos_angle), 2400.0) * 0.4;
  return sun_transmittance(sun_dir().y) * frame.sun_direction.w * (disc * 900.0 + glow * 6.0);
}

@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
  let pixel = vec2<i32>(in.clip_position.xy);
  let depth = textureLoad(depth_texture, pixel, 0);
  let ray = view_ray(in.ndc);
  var colour: vec3<f32>;

  if (depth >= 0.999999) {
    // Background: sky, sun, and clouds.
    var sky = sky_radiance(ray);

    if (ray.y < 0.0) {
      // Below the horizon with no geometry (water disabled, or beyond the
      // terrain): darken towards a hazy ground colour.
      sky = mix(sky, sky * 0.35, saturate(-ray.y * 3.0));
    }

    colour = sky + sun_disc(ray) * step(0.0, ray.y);
    let clouds = march_clouds(ray, 1.0e9, in.clip_position.xy);

    if (clouds.transmittance < 0.999) {
      // Distant clouds fade into the haze near the horizon.
      let haze = exp(-clouds.distance / max(frame.atmosphere.z * 1.4, 1.0));
      let cloud_colour = mix(sky * (1.0 - clouds.transmittance), clouds.scatter, haze);
      colour = colour * clouds.transmittance + cloud_colour;
    }

    let mist = atmospheric_fog(ray, 12000.0, in.clip_position.xy, false);
    colour = colour * mist.transmittance + mist.inscatter;
  } else {
    let scene = textureLoad(scene_texture, pixel, 0).rgb;
    let distance = linear_distance(depth, ray);
    let fog = atmospheric_fog(ray, distance, in.clip_position.xy, true);
    colour = scene * fog.transmittance + fog.inscatter;
    let clouds = march_clouds(ray, distance, in.clip_position.xy);

    if (clouds.transmittance < 0.999) {
      colour = colour * clouds.transmittance + clouds.scatter;
    }
  }

  return vec4<f32>(finish_colour(colour), 1.0);
}
