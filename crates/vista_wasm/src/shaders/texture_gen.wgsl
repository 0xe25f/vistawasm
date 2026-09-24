// Procedural texture baking, run once on the GPU when the engine starts.
//
// Every texture the renderer uses is generated here from seamlessly
// tiling noise: no image files ship with the library, and the output is
// identical on every run. Recipe colours are authored as sRGB values and
// stored as-is (shaders decode them to linear), which keeps dark detail in
// eight-bit channels.
//
// Entry points:
// - `gen_terrain`: ten ground materials, albedo + height and
//   normal + occlusion + roughness.
// - `gen_flora`: bark, leaf clusters, conifer needles, palm fronds, fine
//   leaflets, and hanging moss, with alpha.
// - `gen_water`: ripple normals, foam, and height.
// - `gen_noise`: general-purpose tiling 2D noise (weather, macro detail).
// - `gen_cloud`: 3D Perlin-Worley and Worley noise for volumetric clouds
//   and drifting mist.

@group(0) @binding(0) var terrain_albedo_out: texture_storage_2d_array<rgba8unorm, write>;
@group(0) @binding(1) var terrain_normal_out: texture_storage_2d_array<rgba8unorm, write>;
@group(0) @binding(2) var flora_out: texture_storage_2d_array<rgba8unorm, write>;
@group(0) @binding(3) var water_out: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(4) var noise_out: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(5) var cloud_out: texture_storage_3d<rgba8unorm, write>;

const TAU: f32 = 6.2831853;

// --- Hashing and tiling noise ---------------------------------------------

fn pcg(value: u32) -> u32 {
  let state = value * 747796405u + 2891336453u;
  let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
  return (word >> 22u) ^ word;
}

fn wrap(value: i32, period: i32) -> i32 {
  return ((value % period) + period) % period;
}

fn hash2(cell: vec2<i32>, period: i32, seed: u32) -> u32 {
  return hash2a(cell, vec2<i32>(period, period), seed);
}

fn hash2a(cell: vec2<i32>, period: vec2<i32>, seed: u32) -> u32 {
  let x = u32(wrap(cell.x, period.x));
  let y = u32(wrap(cell.y, period.y));
  return pcg(x + pcg(y + pcg(seed)));
}

fn hash3(cell: vec3<i32>, period: i32, seed: u32) -> u32 {
  let x = u32(wrap(cell.x, period));
  let y = u32(wrap(cell.y, period));
  let z = u32(wrap(cell.z, period));
  return pcg(x + pcg(y + pcg(z + pcg(seed))));
}

fn unit(h: u32) -> f32 {
  return f32(h & 0xffffffu) / 16777215.0;
}

fn gradient2(cell: vec2<i32>, period: vec2<i32>, seed: u32) -> vec2<f32> {
  let angle = unit(hash2a(cell, period, seed)) * TAU;
  return vec2<f32>(cos(angle), sin(angle));
}

fn perlin2(p: vec2<f32>, period: i32, seed: u32) -> f32 {
  return perlin2a(p, vec2<i32>(period, period), seed);
}

// Tiling Perlin noise in roughly -1..1; `p` is in lattice units and the
// pattern repeats every `period` cells along each axis.
fn perlin2a(p: vec2<f32>, period: vec2<i32>, seed: u32) -> f32 {
  let i = vec2<i32>(floor(p));
  let f = fract(p);
  let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
  let a = dot(gradient2(i, period, seed), f);
  let b = dot(gradient2(i + vec2<i32>(1, 0), period, seed), f - vec2<f32>(1.0, 0.0));
  let c = dot(gradient2(i + vec2<i32>(0, 1), period, seed), f - vec2<f32>(0.0, 1.0));
  let d = dot(gradient2(i + vec2<i32>(1, 1), period, seed), f - vec2<f32>(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y) * 1.414;
}

// Tiling fractal noise in roughly 0..1 over the unit square.
fn fbm2(uv: vec2<f32>, period: i32, octaves: i32, seed: u32) -> f32 {
  var value = 0.0;
  var amplitude = 0.5;
  var total = 0.0;
  var frequency = period;

  for (var i = 0; i < octaves; i = i + 1) {
    value = value + perlin2(uv * f32(frequency), frequency, seed + u32(i) * 101u) * amplitude;
    total = total + amplitude;
    amplitude = amplitude * 0.5;
    frequency = frequency * 2;
  }

  return value / total * 0.5 + 0.5;
}

// Tiling Worley noise: returns (F1, F2, cell id) in lattice units.
fn worley2(p: vec2<f32>, period: i32, seed: u32) -> vec3<f32> {
  let i = vec2<i32>(floor(p));
  let f = fract(p);
  var f1 = 8.0;
  var f2 = 8.0;
  var id = 0.0;

  for (var y = -1; y <= 1; y = y + 1) {
    for (var x = -1; x <= 1; x = x + 1) {
      let cell = i + vec2<i32>(x, y);
      let h = hash2(cell, period, seed);
      let point = vec2<f32>(f32(x), f32(y)) + vec2<f32>(unit(h), unit(pcg(h))) - f;
      let d = length(point);

      if (d < f1) {
        f2 = f1;
        f1 = d;
        id = unit(pcg(h + 17u));
      } else if (d < f2) {
        f2 = d;
      }
    }
  }

  return vec3<f32>(f1, f2, id);
}

fn gradient3(cell: vec3<i32>, period: i32, seed: u32) -> vec3<f32> {
  let h = hash3(cell, period, seed);
  let z = unit(h) * 2.0 - 1.0;
  let a = unit(pcg(h)) * TAU;
  let r = sqrt(max(1.0 - z * z, 0.0));
  return vec3<f32>(r * cos(a), r * sin(a), z);
}

fn perlin3(p: vec3<f32>, period: i32, seed: u32) -> f32 {
  let i = vec3<i32>(floor(p));
  let f = fract(p);
  let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
  var corners: array<f32, 8>;

  for (var c = 0; c < 8; c = c + 1) {
    let o = vec3<i32>(c & 1, (c >> 1) & 1, (c >> 2) & 1);
    corners[c] = dot(gradient3(i + o, period, seed), f - vec3<f32>(o));
  }

  let x0 = mix(corners[0], corners[1], u.x);
  let x1 = mix(corners[2], corners[3], u.x);
  let x2 = mix(corners[4], corners[5], u.x);
  let x3 = mix(corners[6], corners[7], u.x);
  return mix(mix(x0, x1, u.y), mix(x2, x3, u.y), u.z);
}

fn worley3(p: vec3<f32>, period: i32, seed: u32) -> f32 {
  let i = vec3<i32>(floor(p));
  let f = fract(p);
  var f1 = 8.0;

  for (var z = -1; z <= 1; z = z + 1) {
    for (var y = -1; y <= 1; y = y + 1) {
      for (var x = -1; x <= 1; x = x + 1) {
        let cell = i + vec3<i32>(x, y, z);
        let h = hash3(cell, period, seed);
        let point = vec3<f32>(f32(x), f32(y), f32(z))
          + vec3<f32>(unit(h), unit(pcg(h)), unit(pcg(h + 7u))) - f;
        f1 = min(f1, dot(point, point));
      }
    }
  }

  return sqrt(f1);
}

// Inverted 3-octave Worley fBm: high inside billows, low between them.
fn worley_fbm3(uvw: vec3<f32>, period: i32, seed: u32) -> f32 {
  let a = 1.0 - saturate(worley3(uvw * f32(period), period, seed));
  let b = 1.0 - saturate(worley3(uvw * f32(period * 2), period * 2, seed + 11u));
  let c = 1.0 - saturate(worley3(uvw * f32(period * 4), period * 4, seed + 23u));
  return a * 0.625 + b * 0.25 + c * 0.125;
}

fn remap(value: f32, old_min: f32, old_max: f32, new_min: f32, new_max: f32) -> f32 {
  return new_min + (value - old_min) / max(old_max - old_min, 0.0001) * (new_max - new_min);
}

// Anisotropic "stroke" noise along a skewed integer lattice, which keeps
// the result tiling while breaking the grid alignment of blades/grains.
// Shifting `uv` by a whole tile moves `q` by an integer lattice vector,
// and the noise period along each axis equals its frequency, so the
// pattern stays seamless for any integer skew.
fn strokes(uv: vec2<f32>, skew: vec2<i32>, period: i32, stretch: f32, seed: u32) -> f32 {
  let q = vec2<f32>(uv.x * f32(skew.x) + uv.y * f32(skew.y), uv.y * f32(skew.x) - uv.x * f32(skew.y));
  let span = max(abs(skew.x) + abs(skew.y), 1);
  var frequency = vec2<i32>(max(period / span, 1), max(i32(f32(period / span) / stretch), 1));
  var value = 0.0;
  var amplitude = 0.5;
  var total = 0.0;

  for (var i = 0; i < 3; i = i + 1) {
    value = value + perlin2a(q * vec2<f32>(frequency), frequency, seed + u32(i) * 101u) * amplitude;
    total = total + amplitude;
    amplitude = amplitude * 0.5;
    frequency = frequency * 2;
  }

  return value / total * 0.5 + 0.5;
}

// --- Terrain materials -------------------------------------------------------

fn lush_grass(uv: vec2<f32>) -> vec4<f32> {
  let broad = fbm2(uv, 3, 4, 11u);
  let blades_a = strokes(uv, vec2<i32>(1, 0), 48, 5.0, 12u);
  let blades_b = strokes(uv, vec2<i32>(2, 1), 32, 5.0, 13u);
  let blades_c = strokes(uv, vec2<i32>(1, -2), 32, 5.0, 14u);
  let blades = max(max(blades_a, blades_b), blades_c);
  let clumps = fbm2(uv, 10, 3, 15u);
  let soil = smoothstep(0.33, 0.25, clumps + (blades - 0.5) * 0.3);
  var colour = mix(vec3<f32>(0.13, 0.2, 0.045), vec3<f32>(0.3, 0.42, 0.11), smoothstep(0.35, 0.8, blades));
  colour = mix(colour, colour * vec3<f32>(1.25, 1.1, 0.7), smoothstep(0.55, 0.8, broad) * 0.6);
  let flecks = smoothstep(0.78, 0.86, fbm2(uv, 64, 2, 16u));
  colour = mix(colour, vec3<f32>(0.5, 0.52, 0.2), flecks * 0.5);
  colour = mix(colour, vec3<f32>(0.2, 0.14, 0.08), soil * 0.8);
  return vec4<f32>(colour, saturate(blades * 0.8 + 0.1 - soil * 0.3));
}

fn dry_grass(uv: vec2<f32>) -> vec4<f32> {
  let broad = fbm2(uv, 3, 4, 21u);
  // Averaging (not max-ing) stroke layers avoids a woven cross-hatch.
  let stalks = strokes(uv, vec2<i32>(1, 0), 48, 7.0, 22u) * 0.5
    + strokes(uv, vec2<i32>(3, 1), 40, 7.0, 23u) * 0.3
    + fbm2(uv, 32, 3, 26u) * 0.2;
  let patches = fbm2(uv, 6, 4, 24u);
  let soil = smoothstep(0.42, 0.3, patches);
  var colour = mix(vec3<f32>(0.4, 0.31, 0.16), vec3<f32>(0.72, 0.62, 0.38), smoothstep(0.35, 0.68, stalks));
  colour = colour * (0.85 + broad * 0.3);
  let pebbles = worley2(uv * 20.0, 20, 25u);
  let pebble = smoothstep(0.2, 0.14, pebbles.x) * step(0.88, pebbles.z);
  colour = mix(colour, vec3<f32>(0.5, 0.42, 0.32), soil * 0.85);
  colour = mix(colour, vec3<f32>(0.55, 0.5, 0.45) * (0.8 + pebbles.z * 0.4), pebble);
  return vec4<f32>(colour, saturate(stalks * 0.7 + pebble * 0.5 - soil * 0.2 + 0.1));
}

fn forest_floor(uv: vec2<f32>) -> vec4<f32> {
  let soil_noise = fbm2(uv, 4, 5, 31u);
  var colour = mix(vec3<f32>(0.09, 0.08, 0.055), vec3<f32>(0.2, 0.17, 0.11), soil_noise);
  var height = soil_noise * 0.3;

  // Fallen leaves in two layers of Worley cells.
  for (var layer = 0; layer < 2; layer = layer + 1) {
    let period = 14 + layer * 6;
    let cells = worley2(uv * f32(period) + vec2<f32>(f32(layer) * 0.5), period, 32u + u32(layer));
    let leaf = smoothstep(0.36, 0.28, cells.x) * step(0.55, cells.z);
    let hue = cells.z;
    let leaf_colour = mix(vec3<f32>(0.24, 0.17, 0.09), vec3<f32>(0.36, 0.28, 0.15), hue) * (0.8 + fbm2(uv, 64, 1, 34u) * 0.4);
    colour = mix(colour, leaf_colour, leaf * 0.75);
    height = max(height, leaf * (0.5 + f32(layer) * 0.2));
  }

  let needles = smoothstep(0.7, 0.8, strokes(uv, vec2<i32>(3, 1), 24, 12.0, 35u));
  colour = mix(colour, vec3<f32>(0.27, 0.19, 0.11), needles * 0.6);
  let moss = smoothstep(0.5, 0.66, fbm2(uv, 5, 5, 36u));
  colour = mix(colour, vec3<f32>(0.14, 0.24, 0.05) * (0.8 + fbm2(uv, 40, 2, 37u) * 0.5), moss);
  height = max(height, moss * 0.6 + needles * 0.4);
  let twigs = smoothstep(0.92, 0.97, strokes(uv, vec2<i32>(1, 2), 8, 20.0, 38u));
  colour = mix(colour, vec3<f32>(0.2, 0.13, 0.07), twigs);
  return vec4<f32>(colour, saturate(height + twigs * 0.5));
}

fn sand(uv: vec2<f32>) -> vec4<f32> {
  let broad = fbm2(uv, 3, 4, 41u);
  let warp = fbm2(uv, 4, 3, 42u);
  let ripples = (sin((uv.y * 9.0 + uv.x * 2.0 + warp * 2.6) * TAU) * 0.5 + 0.5) * smoothstep(0.35, 0.65, fbm2(uv, 2, 3, 45u));
  let grain = unit(hash2(vec2<i32>(uv * 512.0), 512, 43u));
  var colour = mix(vec3<f32>(0.66, 0.57, 0.41), vec3<f32>(0.84, 0.77, 0.6), broad);
  colour = colour * (0.95 + ripples * 0.05) * (0.92 + grain * 0.16);
  let shells = worley2(uv * 26.0, 26, 44u);
  let shell = smoothstep(0.12, 0.07, shells.x) * step(0.8, shells.z);
  colour = mix(colour, mix(vec3<f32>(0.3, 0.27, 0.24), vec3<f32>(0.95, 0.92, 0.86), fract(shells.z * 7.0)), shell);
  return vec4<f32>(colour, saturate(ripples * 0.2 + broad * 0.45 + shell * 0.4));
}

// Ridged noise: sharp creases where Perlin noise crosses zero.
fn ridged2(uv: vec2<f32>, period: i32, octaves: i32, seed: u32) -> f32 {
  var value = 0.0;
  var amplitude = 0.5;
  var total = 0.0;
  var frequency = period;

  for (var i = 0; i < octaves; i = i + 1) {
    let n = 1.0 - abs(perlin2(uv * f32(frequency), frequency, seed + u32(i) * 57u));
    value = value + n * n * amplitude;
    total = total + amplitude;
    amplitude = amplitude * 0.5;
    frequency = frequency * 2;
  }

  return value / total;
}

fn rock(uv: vec2<f32>) -> vec4<f32> {
  // Domain-warped fractal relief with sharp ridges, so faces break into
  // irregular facets and ledges rather than cells.
  let warp = vec2<f32>(fbm2(uv, 3, 3, 51u), fbm2(uv + vec2<f32>(0.37, 0.11), 3, 3, 52u)) - 0.5;
  let relief = ridged2(uv + warp * 0.18, 3, 6, 53u);
  let broad = fbm2(uv + warp * 0.3, 2, 5, 54u);
  let grain = fbm2(uv, 48, 3, 55u);
  // Sparse, thin fractures that follow creases of the relief.
  let fracture_noise = abs(perlin2((uv + warp * 0.25) * 7.0, 7, 56u)) + abs(perlin2((uv + warp * 0.1) * 14.0, 14, 57u)) * 0.4;
  let fracture = 1.0 - smoothstep(0.0, 0.045, fracture_noise);
  let strata = sin((uv.y * 11.0 + broad * 3.5 + warp.x * 2.0) * TAU) * 0.5 + 0.5;
  let cool = vec3<f32>(0.36, 0.37, 0.38);
  let warm = vec3<f32>(0.47, 0.44, 0.4);
  var colour = mix(cool, warm, smoothstep(0.35, 0.8, broad));
  colour = colour * (0.72 + relief * 0.45) * (0.93 + grain * 0.14) * (0.95 + strata * 0.08);
  colour = colour * (1.0 - fracture * 0.45);
  let lichen = smoothstep(0.7, 0.78, fbm2(uv, 9, 4, 58u)) * smoothstep(0.35, 0.6, relief);
  colour = mix(colour, vec3<f32>(0.5, 0.5, 0.36), lichen * 0.35);
  let height = saturate(relief * 0.75 + broad * 0.25 - fracture * 0.3);
  return vec4<f32>(colour, height);
}

fn snow(uv: vec2<f32>) -> vec4<f32> {
  let broad = fbm2(uv, 3, 5, 61u);
  let drifts = sin((uv.x * 9.0 + broad * 2.0) * TAU) * 0.5 + 0.5;
  let crust = fbm2(uv, 24, 3, 62u);
  var colour = mix(vec3<f32>(0.74, 0.8, 0.9), vec3<f32>(0.95, 0.96, 0.98), saturate(broad * 0.7 + drifts * 0.3));
  colour = colour * (0.96 + crust * 0.06);
  return vec4<f32>(colour, saturate(broad * 0.6 + drifts * 0.25 + crust * 0.15));
}

fn mud(uv: vec2<f32>) -> vec4<f32> {
  let broad = fbm2(uv, 4, 5, 71u);
  let wet = fbm2(uv, 3, 4, 72u);
  let puddle = smoothstep(0.62, 0.68, wet);
  var colour = mix(vec3<f32>(0.18, 0.13, 0.09), vec3<f32>(0.32, 0.24, 0.16), broad);
  let cracks = worley2(uv * 9.0, 9, 73u);
  let crack = (1.0 - smoothstep(0.0, 0.05, cracks.y - cracks.x)) * (1.0 - wet);
  colour = colour * (1.0 - crack * 0.4);
  colour = mix(colour, vec3<f32>(0.1, 0.08, 0.06), puddle * 0.7);
  return vec4<f32>(colour, saturate(broad * 0.7 - puddle * 0.6 - crack * 0.2 + 0.2));
}

fn volcanic(uv: vec2<f32>) -> vec4<f32> {
  let broad = fbm2(uv, 3, 5, 81u);
  let cells = worley2(uv * 6.0 + vec2<f32>(broad * 0.7), 6, 82u);
  let crack = 1.0 - smoothstep(0.0, 0.08, cells.y - cells.x);
  let vesicles = worley2(uv * 40.0, 40, 83u);
  let pit = smoothstep(0.25, 0.1, vesicles.x);
  let ash = smoothstep(0.55, 0.7, fbm2(uv, 5, 4, 84u));
  var colour = mix(vec3<f32>(0.05, 0.045, 0.045), vec3<f32>(0.16, 0.14, 0.13), broad);
  colour = colour * (1.0 - pit * 0.5) * (1.0 - crack * 0.7);
  colour = mix(colour, vec3<f32>(0.33, 0.31, 0.3) * (0.8 + broad * 0.4), ash);
  // Height is low in cracks, which is where lava glows.
  let height = saturate(0.35 + broad * 0.4 + ash * 0.2 - crack * 0.6 - pit * 0.1);
  return vec4<f32>(colour, height);
}

fn ice(uv: vec2<f32>) -> vec4<f32> {
  // Wind-scoured undulation: long, gentle waves crossed by broad swells.
  let broad = fbm2(uv, 3, 4, 91u);
  let warp = fbm2(uv, 2, 3, 92u);
  let scour = sin((uv.x * 4.0 + uv.y * 1.0 + warp * 1.6) * TAU) * 0.5 + 0.5;
  let height = saturate(broad * 0.6 + scour * 0.3 + fbm2(uv, 16, 3, 93u) * 0.1);
  // Low, scoured hollows show the deep blue of dense glacier ice.
  var colour = mix(vec3<f32>(0.35, 0.6, 0.8), vec3<f32>(0.8, 0.9, 0.97), smoothstep(0.2, 0.75, height));
  // Trapped air bubbles: fine, pale specks.
  let bubbles = worley2(uv * 48.0, 48, 94u);
  let bubble = smoothstep(0.16, 0.06, bubbles.x) * step(0.72, bubbles.z);
  colour = mix(colour, vec3<f32>(0.93, 0.96, 0.99), bubble * 0.6);
  // Thin hairline fractures through the surface.
  let fracture = 1.0 - smoothstep(0.0, 0.03, abs(perlin2((uv + warp * 0.2) * 5.0, 5, 95u)));
  colour = colour * (1.0 - fracture * 0.18);
  return vec4<f32>(colour, height);
}

fn tundra(uv: vec2<f32>) -> vec4<f32> {
  let broad = fbm2(uv, 4, 5, 101u);
  let hummocks = fbm2(uv, 12, 3, 102u);
  // A mosaic of olive and khaki moss.
  var colour = mix(vec3<f32>(0.33, 0.36, 0.22), vec3<f32>(0.46, 0.43, 0.29), smoothstep(0.35, 0.7, broad));
  colour = colour * (0.8 + hummocks * 0.35);
  // Pale lichen rosettes.
  let lichen_cells = worley2(uv * 10.0 + vec2<f32>(broad * 0.4), 10, 103u);
  let lichen = smoothstep(0.42, 0.3, lichen_cells.x) * step(0.55, lichen_cells.z);
  colour = mix(colour, vec3<f32>(0.62, 0.6, 0.45) * (0.9 + fbm2(uv, 40, 2, 104u) * 0.2), lichen * 0.85);
  // Russet dwarf shrub leaves among the moss.
  let shrubs = smoothstep(0.66, 0.74, fbm2(uv, 8, 3, 105u));
  colour = mix(colour, vec3<f32>(0.4, 0.24, 0.14), shrubs * 0.5);
  // Small grey stones pushed up by frost.
  let stones = worley2(uv * 24.0, 24, 106u);
  let stone = smoothstep(0.3, 0.2, stones.x) * step(0.82, stones.z);
  colour = mix(colour, vec3<f32>(0.47, 0.47, 0.45) * (0.8 + stones.z * 0.3), stone);
  let height = saturate(hummocks * 0.5 + broad * 0.25 + lichen * 0.1 + stone * 0.45);
  return vec4<f32>(colour, height);
}

fn material(layer: i32, uv: vec2<f32>) -> vec4<f32> {
  switch layer {
    case 0: { return lush_grass(uv); }
    case 1: { return dry_grass(uv); }
    case 2: { return forest_floor(uv); }
    case 3: { return sand(uv); }
    case 4: { return rock(uv); }
    case 5: { return snow(uv); }
    case 6: { return mud(uv); }
    case 8: { return ice(uv); }
    case 9: { return tundra(uv); }
    default: { return volcanic(uv); }
  }
}

fn material_roughness(layer: i32, height: f32) -> f32 {
  switch layer {
    case 3: { return 0.85; }
    case 4: { return 0.75 - height * 0.15; }
    case 5: { return 0.45; }
    case 6: { return mix(0.2, 0.7, saturate(height * 2.0)); }
    case 7: { return 0.6; }
    // Scoured hollows are polished bare ice; raised ice is weathered.
    case 8: { return mix(0.15, 0.35, smoothstep(0.35, 0.75, height)); }
    case 9: { return 0.85; }
    default: { return 0.9; }
  }
}

fn material_bump(layer: i32) -> f32 {
  switch layer {
    case 4: { return 5.5; }
    case 7: { return 5.0; }
    case 2: { return 3.5; }
    case 5: { return 1.5; }
    case 3: { return 1.6; }
    case 8: { return 1.2; }
    default: { return 3.0; }
  }
}

@compute @workgroup_size(8, 8, 1)
fn gen_terrain(@builtin(global_invocation_id) id: vec3<u32>) {
  let size = textureDimensions(terrain_albedo_out);

  if (id.x >= size.x || id.y >= size.y) {
    return;
  }

  let layer = i32(id.z);
  let texel = 1.0 / f32(size.x);
  let uv = (vec2<f32>(id.xy) + 0.5) * texel;
  let centre = material(layer, uv);
  let right = material(layer, fract(uv + vec2<f32>(texel, 0.0))).a;
  let up = material(layer, fract(uv + vec2<f32>(0.0, texel))).a;
  let bump = material_bump(layer);
  let normal = normalize(vec3<f32>((centre.a - right) * bump, 1.0, (centre.a - up) * bump));
  let occlusion = saturate(0.55 + centre.a * 0.6);
  textureStore(terrain_albedo_out, vec2<i32>(id.xy), layer, vec4<f32>(saturate(centre.rgb), centre.a));
  textureStore(
    terrain_normal_out,
    vec2<i32>(id.xy),
    layer,
    vec4<f32>(normal.x * 0.5 + 0.5, normal.z * 0.5 + 0.5, occlusion, material_roughness(layer, centre.a))
  );
}

// --- Flora -------------------------------------------------------------------

fn segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
  let ab = b - a;
  let t = saturate(dot(p - a, ab) / max(dot(ab, ab), 1e-6));
  return vec2<f32>(length(p - (a + ab * t)), t);
}

fn bark_oak(uv: vec2<f32>) -> vec4<f32> {
  let warp = fbm2(uv, 3, 3, 101u);
  let ridges = worley2(vec2<f32>(uv.x * 7.0, uv.y * 2.0 + warp * 0.8), 7, 102u);
  let fissure = smoothstep(0.0, 0.18, ridges.y - ridges.x);
  let grain = fbm2(vec2<f32>(uv.x, uv.y * 0.25), 24, 3, 103u);
  var colour = mix(vec3<f32>(0.07, 0.055, 0.045), vec3<f32>(0.34, 0.3, 0.25), fissure);
  colour = colour * (0.8 + grain * 0.4);
  let lichen = smoothstep(0.66, 0.72, fbm2(uv, 6, 3, 104u)) * fissure;
  colour = mix(colour, vec3<f32>(0.42, 0.48, 0.32), lichen * 0.6);
  return vec4<f32>(colour, fissure);
}

fn bark_pine(uv: vec2<f32>) -> vec4<f32> {
  let warp = fbm2(uv, 4, 3, 111u);
  let plates = worley2(vec2<f32>(uv.x * 8.0 + warp * 0.5, uv.y * 6.0), 8, 112u);
  let gap = smoothstep(0.0, 0.07, plates.y - plates.x);
  let flake = fbm2(uv, 32, 3, 113u);
  var colour = mix(vec3<f32>(0.42, 0.25, 0.15), vec3<f32>(0.56, 0.37, 0.24), plates.z);
  colour = colour * (0.8 + flake * 0.35) * (0.85 + smoothstep(0.0, 0.3, plates.y - plates.x) * 0.2);
  colour = mix(vec3<f32>(0.14, 0.09, 0.06), colour, gap);
  return vec4<f32>(colour, gap);
}

fn bark_palm(uv: vec2<f32>) -> vec4<f32> {
  let ring_warp = fbm2(uv, 4, 3, 121u) * 0.25;
  let rings = fract(uv.y * 10.0 + ring_warp);
  let ring = smoothstep(0.0, 0.12, rings) * smoothstep(1.0, 0.8, rings);
  let fibres = fbm2(vec2<f32>(uv.x * 4.0, uv.y * 0.5), 12, 3, 122u);
  var colour = mix(vec3<f32>(0.2, 0.17, 0.13), vec3<f32>(0.5, 0.44, 0.35), ring);
  colour = colour * (0.8 + fibres * 0.4);
  return vec4<f32>(colour, ring);
}

fn bark_smooth(uv: vec2<f32>) -> vec4<f32> {
  let mottle = fbm2(uv, 4, 5, 131u);
  let lenticels = smoothstep(0.8, 0.9, strokes(uv, vec2<i32>(0, 1), 24, 0.2, 132u));
  var colour = mix(vec3<f32>(0.4, 0.38, 0.33), vec3<f32>(0.62, 0.6, 0.53), mottle);
  let algae = smoothstep(0.6, 0.75, fbm2(uv, 3, 4, 133u));
  colour = mix(colour, vec3<f32>(0.34, 0.42, 0.26), algae * 0.5);
  colour = colour * (1.0 - lenticels * 0.35);
  return vec4<f32>(colour, saturate(0.7 + mottle * 0.3 - lenticels * 0.4));
}

// A rounded cluster of ovate leaves radiating from the centre of the card.
fn leaf_cluster(uv: vec2<f32>, count: i32, leaf_length: f32, leaf_width: f32, base: vec3<f32>, tip: vec3<f32>, seed: u32, veined: bool) -> vec4<f32> {
  var result = vec4<f32>(0.0);
  let centre = vec2<f32>(0.5, 0.5);

  for (var i = 0; i < count; i = i + 1) {
    let h = pcg(seed + u32(i) * 7919u);
    let angle = unit(h) * TAU;
    let reach = sqrt(unit(pcg(h))) * (0.46 - leaf_length * 0.5);
    let size = leaf_length * (0.75 + unit(pcg(h + 3u)) * 0.5);
    let direction = vec2<f32>(cos(angle + (unit(pcg(h + 5u)) - 0.5) * 0.9), sin(angle + (unit(pcg(h + 5u)) - 0.5) * 0.9));
    let leaf_base = centre + vec2<f32>(cos(angle), sin(angle)) * reach;
    let local = uv - leaf_base;
    let along = dot(local, direction) / size;
    let across = dot(local, vec2<f32>(-direction.y, direction.x)) / (size * leaf_width);

    if (along < 0.0 || along > 1.0) {
      continue;
    }

    // Ovate outline: widest a third of the way along, pointed tip.
    let half_width = sin(pow(along, 0.75) * 3.14159) * 0.5;

    if (abs(across) < half_width) {
      let shade = 0.75 + 0.25 * (1.0 - abs(across) / max(half_width, 0.001));
      var colour = mix(base, tip, unit(pcg(h + 9u))) * shade;
      let midrib = 1.0 - smoothstep(0.0, 0.05, abs(across));
      colour = mix(colour, colour * 1.35, midrib * 0.6);

      if (veined) {
        let veins = smoothstep(0.75, 1.0, sin((along * 9.0 - abs(across) * 3.0) * TAU * 0.5));
        colour = colour * (1.0 - veins * 0.12);
      }

      // Leaves closer to the twig are a little darker (self-shading).
      colour = colour * (0.75 + 0.25 * saturate(reach / 0.3 + along * 0.5));
      result = vec4<f32>(colour, 1.0);
    }
  }

  return result;
}

fn needles(uv: vec2<f32>) -> vec4<f32> {
  // Main twig along the card's v axis with alternating side twigs.
  var result = vec4<f32>(0.0);
  var twigs = array<vec4<f32>, 9>(
    vec4<f32>(0.5, 0.0, 0.5, 1.0),
    vec4<f32>(0.5, 0.15, 0.2, 0.42),
    vec4<f32>(0.5, 0.25, 0.8, 0.55),
    vec4<f32>(0.5, 0.36, 0.22, 0.64),
    vec4<f32>(0.5, 0.46, 0.78, 0.74),
    vec4<f32>(0.5, 0.56, 0.26, 0.8),
    vec4<f32>(0.5, 0.66, 0.74, 0.86),
    vec4<f32>(0.5, 0.75, 0.32, 0.92),
    vec4<f32>(0.5, 0.83, 0.68, 0.96)
  );

  for (var i = 0; i < 9; i = i + 1) {
    let a = twigs[i].xy;
    let b = twigs[i].zw;
    let axis = normalize(b - a);
    let normal = vec2<f32>(-axis.y, axis.x);
    let local = uv - a;
    let along = dot(local, axis);
    let across = dot(local, normal);
    let twig_length = length(b - a);
    let needle_length = select(0.1, 0.075, i > 0) * (1.0 - saturate(along / twig_length) * 0.4);

    if (along < -0.02 || along > twig_length + needle_length || abs(across) > needle_length) {
      continue;
    }

    let twig_hit = segment_distance(uv, a, b);

    // A dark, ragged mass of distant needles behind the individual ones,
    // so sprays keep their volume in lower mips instead of thinning out.
    let mass_edge = needle_length * (0.2 + fbm2(uv, 32, 3, 142u + u32(i)) * 0.55);

    if (twig_hit.x < mass_edge && along > 0.0 && along < twig_length + needle_length * 0.5 && result.a < 0.5) {
      result = vec4<f32>(vec3<f32>(0.04, 0.09, 0.045) * (0.8 + fbm2(uv, 64, 2, 143u) * 0.4), 1.0);
    }

    if (twig_hit.x < 0.006 * (1.0 - twig_hit.y * 0.6)) {
      result = vec4<f32>(0.2, 0.13, 0.08, 1.0);
    }

    // Needles leave the twig at about 55 degrees, pointing towards its tip.
    let slant = 0.7;
    let spacing = 0.011;
    let start = along - abs(across) * slant;
    let k = round(start / spacing);
    let needle_base = k * spacing;

    if (needle_base >= -0.005 && needle_base <= twig_length) {
      let side = sign(across);
      let direction = normalize(vec2<f32>(slant, 1.0));
      let rel = vec2<f32>(along - needle_base, abs(across));
      let t = dot(rel, direction);
      let distance = abs(rel.x * direction.y - rel.y * direction.x);

      let h = unit(hash2(vec2<i32>(i32(k), i32(side) + i * 3), 4096, 141u));

      // Ragged needle lengths give the spray a soft, feathered outline.
      if (t > 0.0 && t < needle_length * (0.55 + h * 0.45) && distance < 0.0034 * (1.0 - t / needle_length * 0.6)) {
        let colour = mix(vec3<f32>(0.05, 0.12, 0.06), vec3<f32>(0.16, 0.28, 0.12), t / needle_length * 0.7 + h * 0.3);
        result = vec4<f32>(colour, 1.0);
      }
    }
  }

  return result;
}

fn palm_frond(uv: vec2<f32>) -> vec4<f32> {
  let rib_width = 0.018 * (1.0 - uv.y * 0.7);
  let rib_distance = abs(uv.x - 0.5);

  if (rib_distance < rib_width) {
    return vec4<f32>(mix(vec3<f32>(0.45, 0.42, 0.22), vec3<f32>(0.3, 0.36, 0.12), uv.y), 1.0);
  }

  if (uv.y < 0.08) {
    return vec4<f32>(0.0);
  }

  // Long leaflets leave the rib angled towards the tip.
  let slant = 0.55;
  let spacing = 0.03;
  let across = rib_distance - rib_width;
  let start = uv.y - across * slant;
  let k = round(start / spacing);
  let leaflet_base = k * spacing;
  let rel = uv.y - (leaflet_base + across * slant);
  let length_along = across / 0.48;
  let width = 0.011 * (1.0 - length_along * 0.8) * (1.0 - uv.y * 0.3);

  if (leaflet_base < 0.06 || abs(rel) > width || length_along > 1.0 - uv.y * 0.35) {
    return vec4<f32>(0.0);
  }

  let h = unit(hash2(vec2<i32>(i32(k), select(0, 1, uv.x > 0.5)), 4096, 151u));

  // A few leaflets are missing or torn, like real weathered fronds.
  if (h < 0.08 || (h > 0.92 && length_along > 0.5)) {
    return vec4<f32>(0.0);
  }

  var colour = mix(vec3<f32>(0.14, 0.3, 0.06), vec3<f32>(0.3, 0.42, 0.1), h);
  colour = mix(colour, vec3<f32>(0.55, 0.5, 0.2), smoothstep(0.7, 1.0, length_along) * 0.5);
  colour = colour * (0.8 + 0.2 * (1.0 - abs(rel) / width));
  return vec4<f32>(colour, 1.0);
}

fn fine_leaves(uv: vec2<f32>) -> vec4<f32> {
  // Feathery bipinnate sprays radiating from the centre.
  var result = vec4<f32>(0.0);

  for (var i = 0; i < 7; i = i + 1) {
    let h = pcg(161u + u32(i) * 131u);
    let angle = f32(i) / 7.0 * TAU + unit(h) * 0.5;
    let a = vec2<f32>(0.5, 0.5) + vec2<f32>(cos(angle), sin(angle)) * 0.04;
    let b = vec2<f32>(0.5, 0.5) + vec2<f32>(cos(angle), sin(angle)) * (0.36 + unit(pcg(h)) * 0.1);
    let axis = normalize(b - a);
    let local = uv - a;
    let along = dot(local, axis);
    let across = dot(local, vec2<f32>(-axis.y, axis.x));
    let spray_length = length(b - a);

    if (along < 0.0 || along > spray_length + 0.05 || abs(across) > 0.09) {
      continue;
    }

    let rachis = segment_distance(uv, a, b);

    if (rachis.x < 0.004) {
      result = vec4<f32>(0.25, 0.22, 0.1, 1.0);
    }

    let spacing = 0.016;
    let k = round(along / spacing);
    let leaflet_centre = vec2<f32>(k * spacing, sign(across) * 0.035 * (1.0 - k * spacing / spray_length * 0.5));
    let d = (vec2<f32>(along, across) - leaflet_centre) / vec2<f32>(0.007, 0.03);

    if (k * spacing <= spray_length && dot(d, d) < 1.0) {
      let tone = unit(hash2(vec2<i32>(i32(k), i), 4096, 162u));
      result = vec4<f32>(mix(vec3<f32>(0.15, 0.26, 0.06), vec3<f32>(0.3, 0.4, 0.12), tone) * (0.8 + (1.0 - dot(d, d)) * 0.3), 1.0);
    }
  }

  return result;
}

fn moss(uv: vec2<f32>) -> vec4<f32> {
  var result = vec4<f32>(0.0);

  for (var i = 0; i < 14; i = i + 1) {
    let h = pcg(171u + u32(i) * 53u);
    let x = 0.08 + unit(h) * 0.84;
    let strand_length = 0.45 + unit(pcg(h)) * 0.55;
    let hang = 1.0 - uv.y;

    if (hang > strand_length) {
      continue;
    }

    let sway = sin(hang * 9.0 + f32(i)) * 0.03 * hang + sin(hang * 23.0 + f32(i) * 2.0) * 0.01;
    let width = 0.013 * (1.0 - hang / strand_length * 0.7);
    let distance = abs(uv.x - x - sway);

    if (distance < width) {
      let tone = unit(pcg(h + 11u));
      let colour = mix(vec3<f32>(0.4, 0.44, 0.34), vec3<f32>(0.6, 0.63, 0.5), tone) * (0.8 + 0.2 * (1.0 - distance / width));
      result = vec4<f32>(colour, 1.0);
    }
  }

  return result;
}

fn flora(layer: i32, uv: vec2<f32>) -> vec4<f32> {
  switch layer {
    case 0: { return bark_oak(uv); }
    case 1: { return bark_pine(uv); }
    case 2: { return bark_palm(uv); }
    case 3: { return bark_smooth(uv); }
    case 4: { return leaf_cluster(uv, 42, 0.17, 0.5, vec3<f32>(0.1, 0.2, 0.04), vec3<f32>(0.24, 0.36, 0.08), 181u, false); }
    case 5: { return leaf_cluster(uv, 11, 0.38, 0.32, vec3<f32>(0.05, 0.16, 0.035), vec3<f32>(0.14, 0.3, 0.06), 191u, true); }
    case 6: { return needles(uv); }
    case 7: { return palm_frond(uv); }
    case 8: { return fine_leaves(uv); }
    default: { return moss(uv); }
  }
}

@compute @workgroup_size(8, 8, 1)
fn gen_flora(@builtin(global_invocation_id) id: vec3<u32>) {
  let size = textureDimensions(flora_out);

  if (id.x >= size.x || id.y >= size.y) {
    return;
  }

  let layer = i32(id.z);
  // Supersample 2x2 so thin needles and leaflets are anti-aliased.
  var sum = vec4<f32>(0.0);
  var colour_sum = vec3<f32>(0.0);

  for (var sy = 0; sy < 2; sy = sy + 1) {
    for (var sx = 0; sx < 2; sx = sx + 1) {
      let uv = (vec2<f32>(id.xy) + vec2<f32>(0.25 + f32(sx) * 0.5, 0.25 + f32(sy) * 0.5)) / vec2<f32>(size);
      let sample = flora(layer, uv);
      colour_sum = colour_sum + sample.rgb * select(sample.a, 1.0, layer < 4);
      sum = sum + sample;
    }
  }

  var alpha = sum.a / 4.0;
  var colour = colour_sum / max(select(sum.a, 4.0, layer < 4), 0.0001);

  if (layer < 4) {
    // Bark keeps its alpha channel as a cavity/height term.
    alpha = sum.a / 4.0;
  }

  textureStore(flora_out, vec2<i32>(id.xy), layer, vec4<f32>(saturate(colour), alpha));
}

// --- Water ---------------------------------------------------------------------

fn water_height(uv: vec2<f32>) -> f32 {
  let a = fbm2(uv + vec2<f32>(fbm2(uv, 2, 2, 201u) * 0.3), 4, 4, 202u);
  let b = strokes(uv, vec2<i32>(2, 1), 6, 2.5, 203u);
  let c = fbm2(uv, 16, 3, 204u);
  return a * 0.5 + b * 0.3 + c * 0.2;
}

@compute @workgroup_size(8, 8, 1)
fn gen_water(@builtin(global_invocation_id) id: vec3<u32>) {
  let size = textureDimensions(water_out);

  if (id.x >= size.x || id.y >= size.y) {
    return;
  }

  let texel = 1.0 / f32(size.x);
  let uv = (vec2<f32>(id.xy) + 0.5) * texel;
  let h = water_height(uv);
  let hx = water_height(fract(uv + vec2<f32>(texel, 0.0)));
  let hy = water_height(fract(uv + vec2<f32>(0.0, texel)));
  let normal = normalize(vec3<f32>((h - hx) * 60.0, 1.0, (h - hy) * 60.0));
  let bubbles = worley2(uv * 18.0, 18, 205u);
  let streaks = fbm2(uv, 6, 4, 206u);
  let foam = saturate(smoothstep(0.35, 0.05, bubbles.x) * 0.6 + smoothstep(0.5, 0.75, streaks) * 0.7);
  textureStore(water_out, vec2<i32>(id.xy), vec4<f32>(normal.x * 0.5 + 0.5, normal.z * 0.5 + 0.5, foam, h));
}

// --- General noise -----------------------------------------------------------

@compute @workgroup_size(8, 8, 1)
fn gen_noise(@builtin(global_invocation_id) id: vec3<u32>) {
  let size = textureDimensions(noise_out);

  if (id.x >= size.x || id.y >= size.y) {
    return;
  }

  let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(size);
  let r = saturate(remap(fbm2(uv, 4, 6, 301u), 0.22, 0.78, 0.0, 1.0));
  let g = saturate(remap(fbm2(uv, 8, 5, 302u), 0.25, 0.75, 0.0, 1.0));
  let b = 1.0 - saturate(worley2(uv * 6.0, 6, 303u).x);
  let a = saturate(remap(fbm2(uv, 16, 4, 304u), 0.25, 0.75, 0.0, 1.0));
  textureStore(noise_out, vec2<i32>(id.xy), vec4<f32>(r, g, b, a));
}

// --- Cloud noise -------------------------------------------------------------

@compute @workgroup_size(4, 4, 4)
fn gen_cloud(@builtin(global_invocation_id) id: vec3<u32>) {
  let size = textureDimensions(cloud_out);

  if (id.x >= size.x || id.y >= size.y || id.z >= size.z) {
    return;
  }

  let uvw = (vec3<f32>(id) + 0.5) / vec3<f32>(size);
  var perlin = 0.0;
  var amplitude = 0.5;
  var period = 4;

  for (var i = 0; i < 4; i = i + 1) {
    perlin = perlin + perlin3(uvw * f32(period), period, 401u + u32(i)) * amplitude;
    amplitude = amplitude * 0.5;
    period = period * 2;
  }

  perlin = saturate(perlin * 0.9 + 0.5);
  let worley = worley_fbm3(uvw, 4, 411u);
  // Perlin-Worley: Perlin's connectedness with Worley's billowy shapes.
  let base = saturate(remap(perlin, worley - 1.0, 1.0, 0.0, 1.0));
  let g = worley_fbm3(uvw, 4, 421u);
  let b = worley_fbm3(uvw, 8, 431u);
  let a = worley_fbm3(uvw, 12, 441u);
  textureStore(cloud_out, vec3<i32>(id), vec4<f32>(base, g, b, a));
}
