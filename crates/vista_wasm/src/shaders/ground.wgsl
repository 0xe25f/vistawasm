// Ground heights, shared by every render shader (through `common.wgsl`)
// and the tree cull pass, which binds the height texture itself. Every
// function takes the height texture and the terrain mapping as
// arguments, so this file declares no bindings:
//
// - `terrain`: xy terrain half extents (metres), zw metres per texel;
// - `terrain2`: xy height texture size, z 1 when a terrain is loaded,
//   w the terrain's sea level;
// - `mesh`: xy the drawn terrain mesh's centre sample, z metres per
//   sample (0 when no mesh is drawn), w how far the mesh builds the
//   skirt beyond the footprint, in samples.
//
// The CPU mirrors live in `render/terrain_mesh.rs`.

// A density mask texel read as unorm (`painted::density_multiplier`): 0
// none, 128 unchanged, 255 twice as dense.
fn density_multiplier(value: f32) -> f32 {
  let byte = round(value * 255.0);
  return select(byte / 128.0, 1.0 + (byte - 128.0) / 127.0, byte > 128.0);
}

fn hash12(p: vec2<f32>) -> f32 {
  var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
  p3 = p3 + dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

// Beyond the terrain footprint the ground continues as a skirt: from the
// edge height it descends to 60 m below sea level over `SKIRT_METRES`,
// then keeps falling gently as deep sea floor, so the world never ends in
// a wall and the ocean beyond it has a real coast and depth. The terrain
// mesh builds the same skirt on the CPU (`render/terrain_mesh.rs`).
const SKIRT_METRES: f32 = 1500.0;

fn skirt_height(edge: f32, sea: f32, distance: f32, noise: f32) -> f32 {
  let foot = min(edge, sea - 60.0);

  if (distance >= SKIRT_METRES) {
    return foot - (distance - SKIRT_METRES) * 0.08;
  }

  let s = smoothstep(0.0, 1.0, distance / SKIRT_METRES);
  return mix(edge, foot, s) + (edge - foot) * 0.6 * noise * s * (1.0 - s);
}

// Value noise from -1 to 1 with a 700 m wavelength that varies the skirt.
fn skirt_noise(xz: vec2<f32>) -> f32 {
  let p = xz / 700.0;
  let cell = floor(p);
  let f = p - cell;
  let u = f * f * (3.0 - 2.0 * f);
  let top = mix(hash12(cell), hash12(cell + vec2<f32>(1.0, 0.0)), u.x);
  let bottom = mix(hash12(cell + vec2<f32>(0.0, 1.0)), hash12(cell + vec2<f32>(1.0, 1.0)), u.x);
  return mix(top, bottom, u.y) * 2.0 - 1.0;
}

// Bilinear height at a texel position, clamped to the texture.
fn texel_height(heights: texture_2d<f32>, terrain2: vec4<f32>, texel: vec2<f32>) -> f32 {
  let clamped = clamp(texel, vec2<f32>(0.0), terrain2.xy - vec2<f32>(1.001));
  let base = floor(clamped);
  let f = clamped - base;
  let i = vec2<i32>(base);
  let h00 = textureLoad(heights, i, 0).r;
  let h10 = textureLoad(heights, i + vec2<i32>(1, 0), 0).r;
  let h01 = textureLoad(heights, i + vec2<i32>(0, 1), 0).r;
  let h11 = textureLoad(heights, i + vec2<i32>(1, 1), 0).r;
  return mix(mix(h00, h10, f.x), mix(h01, h11, f.x), f.y);
}

// Ground height at a world position: bilinear inside the footprint, the
// skirt beyond it. `skirt_ground` in `render/terrain_mesh.rs`.
fn ground_height(heights: texture_2d<f32>, terrain: vec4<f32>, terrain2: vec4<f32>, xz: vec2<f32>) -> f32 {
  if (terrain2.z < 0.5) {
    return -100000.0;
  }

  let h = texel_height(heights, terrain2, (xz + terrain.xy) / terrain.zw);
  let outside = length(max(abs(xz) - terrain.xy, vec2<f32>(0.0)));

  if (outside <= 0.0) {
    return h;
  }

  return skirt_height(h, terrain2.w, outside, skirt_noise(xz));
}

// Grid steps per LOD band of the terrain mesh (`LOD_BAND_WIDTH`).
const LOD_BAND_WIDTH: f32 = 24.0;

// `band_sample_offset`: the sample offset of the vertex `grid` steps from
// the mesh centre. The step doubles every band: band k starts at
// 24 (2^k - 1) samples, in steps of 2^k.
fn band_offset(grid: f32) -> f32 {
  let g = abs(grid);
  let band = floor(g / LOD_BAND_WIDTH);
  let step = exp2(band);
  return sign(grid) * (LOD_BAND_WIDTH * (step - 1.0) + (g - LOD_BAND_WIDTH * band) * step);
}

// `band_grid`: the grid index of the vertex at or below a non-negative
// sample offset from the mesh centre.
fn band_grid(offset: f32) -> f32 {
  var band = floor(log2(offset / LOD_BAND_WIDTH + 1.0));
  // log2 may round across a band's first vertex; step back or on.
  band = band - select(0.0, 1.0, offset < LOD_BAND_WIDTH * (exp2(band) - 1.0));
  band = band + select(0.0, 1.0, offset >= LOD_BAND_WIDTH * (exp2(band + 1.0) - 1.0));
  let step = exp2(band);
  return LOD_BAND_WIDTH * band + floor((offset - LOD_BAND_WIDTH * (step - 1.0)) / step);
}

// `mesh_cell`: the lower and upper samples of the mesh cell holding
// `sample` along one axis, clamped as the builder clamps its vertices.
fn mesh_cell(sample: f32, centre: f32, last: f32, reach: f32) -> vec2<f32> {
  let offset = sample - centre;
  var grid = band_grid(abs(offset));

  if (offset < 0.0) {
    grid = -grid - 1.0;
  }

  let low = clamp(centre + band_offset(grid), -reach, last + reach);
  let high = clamp(centre + band_offset(grid + 1.0), -reach, last + reach);

  // On the footprint's first or last row or column, take the square
  // inside it: both meet there, and the one inside has no skirt corner.
  if (high > last && sample == low && low == last && last > 0.0) {
    return vec2<f32>(clamp(centre + band_offset(grid - 1.0), -reach, last + reach), low);
  }

  if (low < 0.0 && sample == high && high == 0.0 && last > 0.0) {
    return vec2<f32>(high, clamp(centre + band_offset(grid + 2.0), -reach, last + reach));
  }

  return vec2<f32>(low, high);
}

// Height of the terrain mesh being drawn at a world position: the
// triangle it draws there, in whichever LOD band holds it, including the
// skirt. Each grid square is split from its bottom-left to its top-right
// corner. Where no mesh is drawn, the plain ground. Follows
// `mesh_surface_height_at_sample` in `render/terrain_mesh.rs`.
fn mesh_height(
  heights: texture_2d<f32>,
  terrain: vec4<f32>,
  terrain2: vec4<f32>,
  mesh: vec4<f32>,
  xz: vec2<f32>,
) -> f32 {
  let drawn = mesh.z > 0.0 && terrain2.z > 0.5;
  let metres = select(max(terrain.z, 0.001), mesh.z, drawn);
  let last = round(terrain.xy * 2.0 / metres);
  let sample = xz / metres + last * 0.5;
  let cell_x = mesh_cell(sample.x, mesh.x, last.x, mesh.w);
  let cell_z = mesh_cell(sample.y, mesh.y, last.y, mesh.w);
  // Outside the drawn mesh there is no triangle, only the skirt.
  let inside = drawn && cell_x.x < cell_x.y && cell_z.x < cell_z.y && sample.x >= cell_x.x
    && sample.x <= cell_x.y && sample.y >= cell_z.x && sample.y <= cell_z.y;
  var t = (sample - vec2<f32>(cell_x.x, cell_z.x)) / max(vec2<f32>(cell_x.y - cell_x.x, cell_z.y - cell_z.x), vec2<f32>(1.0));
  t = select(vec2<f32>(0.0), t, inside);
  // The triangle's corners: bottom-left, top-right, then top-left or
  // bottom-right, fetched in one loop so the vertex code exists once;
  // outside the mesh, the point itself.
  let upper = t.x + t.y > 1.0;
  var corners = array<vec2<f32>, 3>(
    vec2<f32>(cell_x.x, cell_z.y),
    vec2<f32>(cell_x.y, cell_z.x),
    select(vec2<f32>(cell_x.x, cell_z.x), vec2<f32>(cell_x.y, cell_z.y), upper),
  );
  var h = array<f32, 3>(0.0, 0.0, 0.0);
  let stride = terrain.z / metres;

  for (var k = 0; k < 3; k = k + 1) {
    let corner = select(sample, corners[k], inside);
    let on_map = inside && all(corner >= vec2<f32>(0.0)) && all(corner <= last);

    // A vertex takes its sample's own height inside the footprint (the
    // nearest texels where a large map's height texture is downsampled),
    // and the skirt's beyond it.
    if (on_map && stride < 1.5) {
      h[k] = textureLoad(heights, vec2<i32>(round(corner)), 0).r;
    } else if (on_map) {
      h[k] = texel_height(heights, terrain2, corner / stride);
    } else {
      h[k] = ground_height(heights, terrain, terrain2, (corner - last * 0.5) * metres);
    }
  }

  let bottom_left = h[0];
  let top_right = h[1];

  if (!upper) {
    let top_left = h[2];
    return top_left + (top_right - top_left) * t.x + (bottom_left - top_left) * t.y;
  }

  let bottom_right = h[2];
  return bottom_right + (bottom_left - bottom_right) * (1.0 - t.x) + (top_right - bottom_right) * (1.0 - t.y);
}

// The height a tree stands at: the lowest drawn ground at the trunk and a
// root radius out along +x, +z, -x and -z, less 5 % of that radius. The
// downhill side then meets the ground and the uphill side is buried
// slightly, as real root flares are. Trees stay upright. With no roots,
// the drawn ground itself. `grounded_base` in `render/flora.rs`.
fn grounded_base(
  heights: texture_2d<f32>,
  terrain: vec4<f32>,
  terrain2: vec4<f32>,
  mesh: vec4<f32>,
  xz: vec2<f32>,
  root: f32,
) -> f32 {
  var lowest = 1e30;
  let points = select(5, 1, root <= 0.0);

  for (var point = 0; point < points; point = point + 1) {
    let angle = f32(point) * 1.5707964;
    let out = select(round(vec2<f32>(cos(angle), sin(angle))), vec2<f32>(0.0), point == 4 || points == 1);
    lowest = min(lowest, mesh_height(heights, terrain, terrain2, mesh, xz + out * root));
  }

  return lowest - 0.05 * root;
}
