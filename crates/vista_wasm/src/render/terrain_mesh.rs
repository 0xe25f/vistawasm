//! CPU-baked terrain mesh construction for the GPU renderer.
//!
//! [`build_terrain_mesh`] bakes a single uniform-resolution mesh covering
//! the whole heightmap; it is downsampled so very large terrain still fits
//! comfortably inside WebGPU buffer limits.
//!
//! [`build_terrain_mesh_centred`] implements the engine's LOD strategy: a
//! single regular grid, recentred on the camera each time it drifts far
//! enough, whose sample spacing grows exponentially with distance from the
//! centre (see [`band_sample_offset`]). Because the mesh stays one
//! connected grid (never split into independent tiers or rings), it can
//! never develop the T-junction cracks that classic multi-tier clipmaps
//! need skirts or stitching indices to hide, while still concentrating
//! detail near the camera and reaching far beyond what a uniform mesh of
//! the same vertex count could cover.

use crate::terrain::biomes::{classify_surface, SurfaceSample, MATERIAL_COUNT};
use crate::terrain::heightmap::HeightMap;
use crate::terrain::normals::generate_normals;
use vista_types::{BiomeOptions, Vec3};

/// One GPU-ready terrain vertex (36 bytes).
///
/// The layout is tightly packed and matches the vertex buffer layout used by
/// `clipmap_render.wgsl`. Position is stored in terrain metres. The normal
/// is octahedron-encoded into two signed 16-bit values, which is far below
/// the precision lighting can show, and the surface material weights are
/// twelve normalised `u8` slots in three `u32`s, unpacked in the shader with
/// `unpack4x8unorm`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TerrainVertex {
  /// World position in terrain metres, centred on the terrain origin.
  pub position: [f32; 3],
  /// Octahedron-encoded unit surface normal.
  pub normal: [i16; 2],
  /// Twelve material weights in `MAT_*` order, four per `u32`, lowest
  /// byte first. Slots 10 and 11 are reserved and always 0.
  pub materials: [u32; 3],
  /// Moisture, temperature, volcanic heat, and ambient occlusion.
  pub climate: [u8; 4],
  /// Biome index, tree cover, river flag, and permanent snow.
  pub biome: [u8; 4],
}

/// Number of material slots in a [`TerrainVertex`].
pub const MATERIAL_SLOTS: usize = 12;

impl TerrainVertex {
  fn new(position: [f32; 3], normal: Vec3, surface: &SurfaceSample) -> Self {
    let mut slots = [0u8; MATERIAL_SLOTS];
    slots[..MATERIAL_COUNT].copy_from_slice(&surface.materials);

    Self {
      position,
      normal: encode_normal(normal),
      materials: pack_material_bytes(&slots),
      climate: [
        surface.moisture,
        surface.temperature,
        surface.heat,
        surface.occlusion,
      ],
      biome: [
        surface.biome,
        surface.forest,
        surface.river,
        surface.permanent_snow,
      ],
    }
  }
}

/// Pack twelve byte weights like WGSL's `pack4x8unorm`: the first of each
/// group of four in the lowest byte.
pub fn pack_material_bytes(bytes: &[u8; MATERIAL_SLOTS]) -> [u32; 3] {
  let word = |start: usize| {
    u32::from_le_bytes([
      bytes[start],
      bytes[start + 1],
      bytes[start + 2],
      bytes[start + 3],
    ])
  };

  [word(0), word(4), word(8)]
}

/// Pack twelve 0 to 1 weights, rounding as `pack4x8unorm` does.
pub fn pack_materials(weights: &[f32; MATERIAL_SLOTS]) -> [u32; 3] {
  let mut bytes = [0u8; MATERIAL_SLOTS];

  for (byte, weight) in bytes.iter_mut().zip(weights) {
    *byte = (weight.clamp(0.0, 1.0) * 255.0).round() as u8;
  }

  pack_material_bytes(&bytes)
}

/// Unpack twelve weights, as `unpack4x8unorm` does in the shader.
pub fn unpack_materials(words: [u32; 3]) -> [f32; MATERIAL_SLOTS] {
  let mut weights = [0.0; MATERIAL_SLOTS];

  for (index, weight) in weights.iter_mut().enumerate() {
    let byte = (words[index / 4] >> ((index % 4) * 8)) & 0xff;
    *weight = byte as f32 / 255.0;
  }

  weights
}

/// Octahedron-encode a unit normal into two snorm16 values. Decoded by
/// `decode_normal` in `clipmap_render.wgsl`.
pub fn encode_normal(normal: Vec3) -> [i16; 2] {
  let length = normal[0].abs() + normal[1].abs() + normal[2].abs();

  if length <= f32::EPSILON {
    return [0, 0];
  }

  let (mut u, mut v) = (normal[0] / length, normal[2] / length);

  // The lower hemisphere folds over the diagonals.
  if normal[1] < 0.0 {
    let (fold_u, fold_v) = ((1.0 - v.abs()) * sign(u), (1.0 - u.abs()) * sign(v));
    u = fold_u;
    v = fold_v;
  }

  let quantise = |value: f32| (value.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
  [quantise(u), quantise(v)]
}

/// Decode a normal written by [`encode_normal`].
pub fn decode_normal(encoded: [i16; 2]) -> Vec3 {
  let u = (encoded[0] as f32 / 32_767.0).max(-1.0);
  let v = (encoded[1] as f32 / 32_767.0).max(-1.0);
  let y = 1.0 - u.abs() - v.abs();
  let (x, z) = if y < 0.0 {
    ((1.0 - v.abs()) * sign(u), (1.0 - u.abs()) * sign(v))
  } else {
    (u, v)
  };

  crate::maths::normalise([x, y, z])
}

fn sign(value: f32) -> f32 {
  if value >= 0.0 {
    1.0
  } else {
    -1.0
  }
}

/// CPU-baked terrain mesh ready for GPU upload.
pub struct TerrainMeshData {
  /// Interleaved vertex data.
  pub vertices: Vec<TerrainVertex>,
  /// Triangle list indices.
  pub indices: Vec<u32>,
}

/// Maximum vertices per mesh axis. Large DEM or fractal terrain is
/// downsampled so the mesh stays inside conservative WebGPU buffer limits.
const MAX_MESH_SAMPLES_PER_SIDE: u32 = 513;

/// Number of grid steps per LOD band before the sample step doubles again.
/// A smaller value concentrates more of the vertex budget close to the
/// centre; a larger value spreads it out more evenly.
const LOD_BAND_WIDTH: i64 = 24;

/// Vertices per side used by the camera-centred LOD mesh.
pub const CENTRED_MESH_SAMPLES_PER_SIDE: u32 = MAX_MESH_SAMPLES_PER_SIDE;

/// Convert a signed grid-index distance from the mesh centre into a signed
/// heightmap-sample offset.
///
/// The first [`LOD_BAND_WIDTH`] grid steps out from the centre each advance
/// the sample position by 1 (full native resolution). The next
/// `LOD_BAND_WIDTH` grid steps each advance it by 2 samples, the next band
/// by 4, and so on, doubling every `LOD_BAND_WIDTH` grid steps. This keeps
/// the mesh a single regular grid (so it is always crack-free) while
/// concentrating detail near the centre and letting a fixed vertex budget
/// reach far beyond what uniform spacing could cover.
fn band_sample_offset(grid_distance: i64) -> i64 {
  let sign = grid_distance.signum();
  let mut remaining = grid_distance.unsigned_abs();
  let mut offset: u64 = 0;
  let mut step: u64 = 1;
  let band_width = LOD_BAND_WIDTH.unsigned_abs();

  while remaining > 0 {
    let take = remaining.min(band_width);
    offset += take * step;
    remaining -= take;
    step *= 2;
  }

  sign * offset as i64
}

/// Build a single-resolution terrain mesh from CPU height, normal, and
/// material data.
pub fn build_terrain_mesh(map: &HeightMap) -> TerrainMeshData {
  let width = map.metadata.width;
  let height = map.metadata.height;

  if width == 0 || height == 0 {
    return TerrainMeshData {
      vertices: Vec::new(),
      indices: Vec::new(),
    };
  }

  let stride = (width.max(height).saturating_sub(1) / (MAX_MESH_SAMPLES_PER_SIDE - 1)).max(1);
  let (normals, surface) = bake_terrain_shading(map, &BiomeOptions::default(), None);

  let samples_x = (width - 1) / stride + 1;
  let samples_y = (height - 1) / stride + 1;
  let metres_per_sample = map.metadata.metres_per_sample.max(0.001);
  let half_width = (width as f32 - 1.0) * metres_per_sample * 0.5;
  let half_height = (height as f32 - 1.0) * metres_per_sample * 0.5;

  let mut vertices = Vec::with_capacity((samples_x * samples_y) as usize);

  for sy in 0..samples_y {
    let y = (sy * stride).min(height - 1);

    for sx in 0..samples_x {
      let x = (sx * stride).min(width - 1);
      let index = (y * width + x) as usize;
      let world_x = x as f32 * metres_per_sample - half_width;
      let world_z = y as f32 * metres_per_sample - half_height;
      vertices.push(TerrainVertex::new(
        [world_x, map.heights[index], world_z],
        normals[index],
        &surface[index],
      ));
    }
  }

  let mut indices =
    Vec::with_capacity((samples_x.saturating_sub(1) * samples_y.saturating_sub(1) * 6) as usize);

  for sy in 0..samples_y.saturating_sub(1) {
    for sx in 0..samples_x.saturating_sub(1) {
      let top_left = sy * samples_x + sx;
      let top_right = top_left + 1;
      let bottom_left = top_left + samples_x;
      let bottom_right = bottom_left + 1;

      indices.push(top_left);
      indices.push(bottom_left);
      indices.push(top_right);
      indices.push(top_right);
      indices.push(bottom_left);
      indices.push(bottom_right);
    }
  }

  TerrainMeshData { vertices, indices }
}

/// Number of distinct LOD bands (sample-step doublings) spanned by a
/// centred mesh whose half-span, in grid steps from the centre, is
/// `half_span_samples`. Used only for render statistics.
pub fn band_count(half_span_samples: u32) -> u32 {
  let mut remaining = half_span_samples as i64;
  let mut bands: u32 = 0;

  while remaining > 0 {
    remaining -= LOD_BAND_WIDTH;
    bands += 1;
  }

  bands.max(1)
}

/// Precompute per-sample normals and biome/surface data for a heightmap.
///
/// Both are relatively expensive full-heightmap passes, so the engine calls
/// this once per active terrain (and again only when biome or river
/// settings change) and reuses the result every time
/// [`build_terrain_mesh_centred`] recentres the LOD mesh on the camera.
pub fn bake_terrain_shading(
  map: &HeightMap,
  biomes: &BiomeOptions,
  river_mask: Option<&[bool]>,
) -> (Vec<Vec3>, Vec<SurfaceSample>) {
  let normals = generate_normals(map);
  let surface = classify_surface(map, &normals, river_mask, biomes);
  (normals, surface)
}

/// Convert a world-space position (in the same terrain-centred metres used
/// by [`build_terrain_mesh`]) into fractional heightmap sample coordinates.
pub fn world_to_sample_coordinates(map: &HeightMap, world_x: f32, world_z: f32) -> (f32, f32) {
  let metres_per_sample = map.metadata.metres_per_sample.max(0.001);
  let half_width = (map.metadata.width as f32 - 1.0) * 0.5;
  let half_height = (map.metadata.height as f32 - 1.0) * 0.5;
  let sample_x = world_x / metres_per_sample + half_width;
  let sample_z = world_z / metres_per_sample + half_height;
  (sample_x, sample_z)
}

/// Build a terrain mesh recentred on `(centre_sample_x, centre_sample_z)`
/// (fractional heightmap sample coordinates), with sample spacing that
/// grows with distance from the centre. See the module documentation and
/// [`band_sample_offset`] for the LOD strategy.
///
/// `normals` and `surface` must be the full per-sample arrays for `map`,
/// as produced by [`bake_terrain_shading`].
pub fn build_terrain_mesh_centred(
  map: &HeightMap,
  normals: &[Vec3],
  surface: &[SurfaceSample],
  centre_sample_x: f32,
  centre_sample_z: f32,
  samples_per_side: u32,
) -> TerrainMeshData {
  let mut vertices = Vec::with_capacity((samples_per_side * samples_per_side) as usize);
  build_centred_mesh_rows(
    map,
    normals,
    surface,
    (centre_sample_x, centre_sample_z),
    samples_per_side,
    0..samples_per_side,
    &mut vertices,
  );

  if vertices.is_empty() {
    return TerrainMeshData {
      vertices,
      indices: Vec::new(),
    };
  }

  TerrainMeshData {
    vertices,
    indices: centred_mesh_indices(samples_per_side),
  }
}

/// Append the vertices of grid rows `rows` of the camera-centred mesh to
/// `out`. Building a few rows at a time lets the engine spread one mesh
/// rebuild over several frames instead of stalling a single frame. Appends
/// nothing when the inputs are empty or inconsistent.
pub fn build_centred_mesh_rows(
  map: &HeightMap,
  normals: &[Vec3],
  surface: &[SurfaceSample],
  centre: (f32, f32),
  samples_per_side: u32,
  rows: std::ops::Range<u32>,
  out: &mut Vec<TerrainVertex>,
) {
  let width = map.metadata.width;
  let height = map.metadata.height;

  if width == 0
    || height == 0
    || samples_per_side < 2
    || surface.len() != normals.len()
    || normals.len() != map.heights.len()
  {
    return;
  }

  let metres_per_sample = map.metadata.metres_per_sample.max(0.001);
  let half_width = (width as f32 - 1.0) * 0.5;
  let half_height = (height as f32 - 1.0) * 0.5;
  let half_span = (samples_per_side / 2) as i64;
  let centre_x = centre.0.round() as i64;
  let centre_z = centre.1.round() as i64;
  // The skirt is built out to `SKIRT_MESH_METRES` beyond the footprint;
  // vertices further out collapse onto that line, as sea floor deep
  // enough to be hidden by the water is not worth drawing.
  let reach = (SKIRT_MESH_METRES / metres_per_sample).ceil() as i64;
  // Grid step, in samples, between a vertex and its outward neighbour.
  let step =
    |grid: i64| (band_sample_offset(grid + grid.signum()) - band_sample_offset(grid)).abs();

  for sz in rows.start..rows.end.min(samples_per_side) {
    let grid_dz = sz as i64 - half_span;
    let offset_z = (centre_z + band_sample_offset(grid_dz))
      .max(-reach)
      .min(height as i64 - 1 + reach);
    let sample_z = offset_z.max(0).min(height as i64 - 1) as u32;

    for sx in 0..samples_per_side {
      let grid_dx = sx as i64 - half_span;
      let offset_x = (centre_x + band_sample_offset(grid_dx))
        .max(-reach)
        .min(width as i64 - 1 + reach);
      let sample_x = offset_x.max(0).min(width as i64 - 1) as u32;
      let index = (sample_z * width + sample_x) as usize;
      let world_x = (offset_x as f32 - half_width) * metres_per_sample;
      let world_z = (offset_z as f32 - half_height) * metres_per_sample;

      if offset_x == sample_x as i64 && offset_z == sample_z as i64 {
        out.push(TerrainVertex::new(
          [world_x, map.heights[index], world_z],
          normals[index],
          &surface[index],
        ));
        continue;
      }

      // Beyond the footprint the vertex keeps its true position on the
      // skirt, rather than collapsing onto the border.
      let spacing = step(grid_dx).max(step(grid_dz)).max(1) as f32 * metres_per_sample * 0.5;
      let ground = |x: f32, z: f32| skirt_ground(map, x, z);
      let height_here = ground(world_x, world_z);
      let normal = crate::maths::normalise([
        ground(world_x - spacing, world_z) - ground(world_x + spacing, world_z),
        2.0 * spacing,
        ground(world_x, world_z - spacing) - ground(world_x, world_z + spacing),
      ]);
      out.push(TerrainVertex::new(
        [world_x, height_here, world_z],
        normal,
        &skirt_surface(
          &surface[index],
          outside_distance(map, world_x, world_z),
          height_here - map.metadata.sea_level_metres,
        ),
      ));
    }
  }
}

/// How far the skirt beyond the terrain footprint takes to descend from
/// the edge to [`SKIRT_DEPTH`] below sea level, in metres. `common.wgsl`
/// declares the same constant.
pub const SKIRT_METRES: f32 = 1500.0;

/// How far beyond the footprint the terrain mesh builds the skirt, in
/// metres. The sea floor there lies 300 m deep, so water less clear than
/// 260 m (`WaterOptions.clarityMetres`) hides everything beyond it.
pub const SKIRT_MESH_METRES: f32 = SKIRT_METRES + (300.0 - SKIRT_DEPTH) / SKIRT_SLOPE;

/// Depth below sea level at the foot of the skirt, in metres.
pub const SKIRT_DEPTH: f32 = 60.0;

/// Fall of the sea floor per metre beyond the skirt.
pub const SKIRT_SLOPE: f32 = 0.08;

/// Wavelength of the skirt's noise, in metres.
const SKIRT_NOISE_METRES: f32 = 700.0;

/// Height of the skirt `distance` metres beyond the terrain footprint,
/// where the edge of the terrain stands at `edge`: a smoothstep from the
/// edge down to [`SKIRT_DEPTH`] below `sea` over [`SKIRT_METRES`], varied
/// by `noise` (-1 to 1) by up to 15 % of the drop, then the gentle slope
/// of the deep sea floor. An edge already deeper than the foot keeps its
/// depth. Mirrors `skirt_height` in `common.wgsl`.
pub fn skirt_height(edge: f32, sea: f32, distance: f32, noise: f32) -> f32 {
  let foot = edge.min(sea - SKIRT_DEPTH);

  if distance >= SKIRT_METRES {
    return foot - (distance - SKIRT_METRES) * SKIRT_SLOPE;
  }

  let t = distance.max(0.0) / SKIRT_METRES;
  let s = t * t * (3.0 - 2.0 * t);
  edge + (foot - edge) * s + (edge - foot) * 0.6 * noise * s * (1.0 - s)
}

/// `hash12` from `common.wgsl`.
fn hash12(x: f32, y: f32) -> f32 {
  let fract = |v: f32| v - v.floor();
  let (a, b, c) = (fract(x * 0.1031), fract(y * 0.1031), fract(x * 0.1031));
  let d = a * (b + 33.33) + b * (c + 33.33) + c * (a + 33.33);
  let (a, b, c) = (a + d, b + d, c + d);
  fract((a + b) * c)
}

/// Smooth value noise from -1 to 1 that varies the skirt. Mirrors
/// `skirt_noise` in `common.wgsl`.
pub fn skirt_noise(x: f32, z: f32) -> f32 {
  let (px, pz) = (x / SKIRT_NOISE_METRES, z / SKIRT_NOISE_METRES);
  let (cx, cz) = (px.floor(), pz.floor());
  let smooth = |v: f32| v * v * (3.0 - 2.0 * v);
  let (u, v) = (smooth(px - cx), smooth(pz - cz));
  let top = crate::maths::lerp(hash12(cx, cz), hash12(cx + 1.0, cz), u);
  let bottom = crate::maths::lerp(hash12(cx, cz + 1.0), hash12(cx + 1.0, cz + 1.0), u);
  crate::maths::lerp(top, bottom, v) * 2.0 - 1.0
}

/// Ground height at a world position, as `terrain_height_at` in
/// `common.wgsl` computes it: bilinear inside the footprint, the skirt
/// beyond it.
#[inline(never)]
pub fn skirt_ground(map: &HeightMap, x: f32, z: f32) -> f32 {
  let width = map.metadata.width as usize;
  let height = map.metadata.height as usize;
  let (sx, sz) = world_to_sample_coordinates(map, x, z);
  let cx = sx.clamp(0.0, width as f32 - 1.001);
  let cz = sz.clamp(0.0, height as f32 - 1.001);
  let (x0, z0) = (cx as usize, cz as usize);
  let (fx, fz) = (cx - x0 as f32, cz - z0 as f32);
  let at = |x: usize, z: usize| map.heights[z * width + x];
  let lerp = crate::maths::lerp;
  let edge = lerp(
    lerp(at(x0, z0), at(x0 + 1, z0), fx),
    lerp(at(x0, z0 + 1), at(x0 + 1, z0 + 1), fx),
    fz,
  );
  let distance = outside_distance(map, x, z);

  if distance <= 0.0 {
    return edge;
  }

  skirt_height(
    edge,
    map.metadata.sea_level_metres,
    distance,
    skirt_noise(x, z),
  )
}

/// Distance in metres from a world position to the terrain footprint.
fn outside_distance(map: &HeightMap, x: f32, z: f32) -> f32 {
  let spacing = map.metadata.metres_per_sample.max(0.001);
  let half_x = (map.metadata.width as f32 - 1.0) * 0.5 * spacing;
  let half_z = (map.metadata.height as f32 - 1.0) * 0.5 * spacing;
  let (dx, dz) = ((x.abs() - half_x).max(0.0), (z.abs() - half_z).max(0.0));
  (dx * dx + dz * dz).sqrt()
}

/// The surface of a skirt vertex: the nearest edge sample's, turning to
/// rock over the first 300 m, and to sand from the waterline down. No
/// trees or rivers grow on it.
fn skirt_surface(edge: &SurfaceSample, distance: f32, above_sea: f32) -> SurfaceSample {
  use crate::terrain::biomes::{MAT_ROCK, MAT_SAND};

  let rock = (distance / 300.0).clamp(0.0, 1.0);
  let sand = ((3.0 - above_sea) / 5.0).clamp(0.0, 1.0);
  let mut materials = [0.0f32; MATERIAL_COUNT];

  for (weight, byte) in materials.iter_mut().zip(edge.materials) {
    *weight = byte as f32 * (1.0 - rock);
  }

  materials[MAT_ROCK] += 255.0 * rock;

  for (index, weight) in materials.iter_mut().enumerate() {
    *weight = *weight * (1.0 - sand) + if index == MAT_SAND { 255.0 * sand } else { 0.0 };
  }

  SurfaceSample {
    materials: materials.map(|weight| weight.round() as u8),
    forest: 0,
    river: 0,
    permanent_snow: (edge.permanent_snow as f32 * (1.0 - rock)) as u8,
    ..*edge
  }
}

/// Triangle indices of the camera-centred mesh. They depend only on the
/// grid size, so they are uploaded once and reused by every rebuild.
pub fn centred_mesh_indices(samples_per_side: u32) -> Vec<u32> {
  let quads_per_side = samples_per_side.saturating_sub(1);
  let mut indices = Vec::with_capacity((quads_per_side * quads_per_side * 6) as usize);

  for sz in 0..quads_per_side {
    for sx in 0..quads_per_side {
      let top_left = sz * samples_per_side + sx;
      let top_right = top_left + 1;
      let bottom_left = top_left + samples_per_side;
      let bottom_right = bottom_left + 1;

      indices.push(top_left);
      indices.push(bottom_left);
      indices.push(top_right);
      indices.push(top_right);
      indices.push(bottom_left);
      indices.push(bottom_right);
    }
  }

  indices
}

/// Drift, in samples, between the camera and the displayed mesh centre
/// that starts building the next mesh in the background.
pub const RECENTRE_START_SAMPLES: f32 = 6.0;

/// Drift at which the next mesh must be finished at once. The first LOD
/// band is 24 samples wide, so this keeps the camera inside full detail.
pub const RECENTRE_LIMIT_SAMPLES: f32 = 18.0;

/// How far ahead, in seconds of travel, the next mesh is centred.
const RECENTRE_LEAD_SECONDS: f32 = 0.5;

/// Furthest ahead of the camera, in samples, the next mesh is centred.
const RECENTRE_MAX_LEAD_SAMPLES: f32 = 8.0;

/// Where to centre the next camera-centred mesh, or `None` while the
/// displayed one is still close enough. Like a chunk streamer that preloads
/// in the direction of travel, the new centre leads the camera by its
/// velocity (in samples per second), so a moving camera flies into detail
/// that is already there and the next rebuild comes later.
pub fn next_mesh_centre(
  camera: (f32, f32),
  velocity: (f32, f32),
  displayed: (f32, f32),
) -> Option<(f32, f32)> {
  let drift = (camera.0 - displayed.0)
    .abs()
    .max((camera.1 - displayed.1).abs());

  if drift <= RECENTRE_START_SAMPLES {
    return None;
  }

  let mut lead = (
    velocity.0 * RECENTRE_LEAD_SECONDS,
    velocity.1 * RECENTRE_LEAD_SECONDS,
  );
  let length = (lead.0 * lead.0 + lead.1 * lead.1).sqrt();

  if length > RECENTRE_MAX_LEAD_SAMPLES {
    let scale = RECENTRE_MAX_LEAD_SAMPLES / length;
    lead = (lead.0 * scale, lead.1 * scale);
  }

  Some((camera.0 + lead.0, camera.1 + lead.1))
}

const _: () = assert!(std::mem::size_of::<TerrainVertex>() == 36);

#[cfg(test)]
mod tests {
  use super::*;
  use vista_types::TerrainMetadata;

  #[test]
  fn twelve_material_weights_round_trip_within_one_step() {
    let mut weights = [0.0; MATERIAL_SLOTS];

    for (index, weight) in weights.iter_mut().enumerate() {
      *weight = (index as f32 * 0.137 + 0.01).fract();
    }

    let unpacked = unpack_materials(pack_materials(&weights));

    for (before, after) in weights.iter().zip(unpacked) {
      assert!((before - after).abs() <= 1.0 / 255.0, "{before} -> {after}");
    }

    let bytes: [u8; MATERIAL_SLOTS] = std::array::from_fn(|index| (index * 20) as u8);
    let words = pack_material_bytes(&bytes);
    assert_eq!(words[0] & 0xff, 0);
    assert_eq!(words[2] >> 24, 220);
    assert_eq!(bytemuck::cast_slice::<u32, u8>(&words), &bytes);
  }

  #[test]
  fn octahedral_normals_round_trip_closely() {
    let normals = [
      [0.0, 1.0, 0.0],
      [0.0, -1.0, 0.0],
      [1.0, 0.0, 0.0],
      [0.0, 0.0, -1.0],
      [0.3, 0.9, -0.2],
      [-0.7, -0.1, 0.6],
      [0.577, -0.577, -0.577],
    ];

    for normal in normals {
      let normal = crate::maths::normalise(normal);
      let decoded = decode_normal(encode_normal(normal));
      let dot = normal[0] * decoded[0] + normal[1] * decoded[1] + normal[2] * decoded[2];
      assert!(dot > 0.999_99, "{normal:?} -> {decoded:?}");
    }
  }

  #[test]
  fn vertices_carry_ice_tundra_and_permanent_snow() {
    let sample = SurfaceSample {
      materials: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
      permanent_snow: 200,
      ..SurfaceSample::default()
    };
    let vertex = TerrainVertex::new([0.0; 3], [0.0, 1.0, 0.0], &sample);
    let bytes: &[u8] = bytemuck::cast_slice(&vertex.materials);

    assert_eq!(&bytes[..10], &sample.materials);
    assert_eq!(&bytes[10..], &[0, 0]);
    assert_eq!(vertex.biome[3], 200);
  }

  #[test]
  fn builds_a_grid_with_expected_triangle_count() {
    let map = HeightMap::flat(4, 4, 10.0, TerrainMetadata::default());
    let mesh = build_terrain_mesh(&map);

    assert_eq!(mesh.vertices.len(), 16);
    assert_eq!(mesh.indices.len(), 3 * 3 * 6);
  }

  #[test]
  fn downsamples_large_terrain_below_the_vertex_cap() {
    let map = HeightMap::flat(2049, 2049, 5.0, TerrainMetadata::default());
    let mesh = build_terrain_mesh(&map);

    assert!(
      mesh.vertices.len() <= (MAX_MESH_SAMPLES_PER_SIDE * MAX_MESH_SAMPLES_PER_SIDE) as usize
    );
  }

  #[test]
  fn rows_built_in_slices_match_the_whole_mesh() {
    let mut map = HeightMap::flat(65, 65, 10.0, TerrainMetadata::default());

    for (index, height) in map.heights.iter_mut().enumerate() {
      *height = (index % 7) as f32;
    }

    let (normals, surface) = bake_terrain_shading(&map, &BiomeOptions::default(), None);
    let whole = build_terrain_mesh_centred(&map, &normals, &surface, 20.0, 40.0, 33);
    let mut sliced = Vec::new();

    for start in (0..33).step_by(8) {
      build_centred_mesh_rows(
        &map,
        &normals,
        &surface,
        (20.0, 40.0),
        33,
        start..(start + 8).min(33),
        &mut sliced,
      );
    }

    assert_eq!(sliced.len(), whole.vertices.len());
    assert!(sliced
      .iter()
      .zip(&whole.vertices)
      .all(|(a, b)| bytemuck::bytes_of(a) == bytemuck::bytes_of(b)));
    assert_eq!(whole.indices, centred_mesh_indices(33));
  }

  #[test]
  fn next_centre_waits_for_drift_then_leads_the_camera() {
    assert_eq!(
      next_mesh_centre((103.0, 100.0), (0.0, 0.0), (100.0, 100.0)),
      None
    );

    let still = next_mesh_centre((110.0, 100.0), (0.0, 0.0), (100.0, 100.0)).unwrap();
    assert_eq!(still, (110.0, 100.0));

    let moving = next_mesh_centre((110.0, 100.0), (4.0, 0.0), (100.0, 100.0)).unwrap();
    assert_eq!(moving, (112.0, 100.0));

    // Very fast travel leads by at most the cap, staying in full detail.
    let fast = next_mesh_centre((110.0, 100.0), (0.0, 1000.0), (100.0, 100.0)).unwrap();
    assert!((fast.1 - 100.0 - RECENTRE_MAX_LEAD_SAMPLES).abs() < 1e-4);
    assert!(RECENTRE_MAX_LEAD_SAMPLES + RECENTRE_START_SAMPLES < LOD_BAND_WIDTH as f32);
  }

  #[test]
  fn band_offset_is_identity_within_the_first_band() {
    for distance in -LOD_BAND_WIDTH..=LOD_BAND_WIDTH {
      assert_eq!(band_sample_offset(distance), distance);
    }
  }

  #[test]
  fn band_offset_doubles_step_after_each_band() {
    // One step into the second band should advance by 2 samples, not 1.
    let at_band_edge = band_sample_offset(LOD_BAND_WIDTH);
    let one_into_next_band = band_sample_offset(LOD_BAND_WIDTH + 1);

    assert_eq!(one_into_next_band - at_band_edge, 2);
  }

  #[test]
  fn band_offset_is_symmetric_and_monotonic() {
    let mut previous = band_sample_offset(0);

    for distance in 1..300 {
      let offset = band_sample_offset(distance);
      assert_eq!(offset, -band_sample_offset(-distance));
      assert!(offset > previous);
      previous = offset;
    }
  }

  #[test]
  fn centred_mesh_reaches_further_than_a_uniform_mesh_of_the_same_size() {
    let map = HeightMap::flat(4096, 4096, 10.0, TerrainMetadata::default());
    let (normals, surface) = bake_terrain_shading(&map, &BiomeOptions::default(), None);
    let mesh = build_terrain_mesh_centred(
      &map,
      &normals,
      &surface,
      2048.0,
      2048.0,
      MAX_MESH_SAMPLES_PER_SIDE,
    );

    assert_eq!(
      mesh.vertices.len(),
      (MAX_MESH_SAMPLES_PER_SIDE * MAX_MESH_SAMPLES_PER_SIDE) as usize
    );

    // The outermost vertex must sample much further from the centre than a
    // uniform-step mesh of the same vertex count would reach.
    let half_span = (MAX_MESH_SAMPLES_PER_SIDE / 2) as i64;
    let farthest_offset = band_sample_offset(half_span);
    assert!(farthest_offset > half_span * 4);
  }

  #[test]
  fn centred_mesh_clamps_to_heightmap_bounds_near_the_edge() {
    let map = HeightMap::flat(64, 64, 1.0, TerrainMetadata::default());
    let (normals, surface) = bake_terrain_shading(&map, &BiomeOptions::default(), None);
    // Centre right at the corner so most of the mesh would fall outside
    // the heightmap without clamping.
    let mesh = build_terrain_mesh_centred(&map, &normals, &surface, 0.0, 0.0, 65);

    assert_eq!(mesh.vertices.len(), 65 * 65);
    assert!(mesh
      .vertices
      .iter()
      .all(|vertex| vertex.position[0].is_finite() && vertex.position[2].is_finite()));
  }

  #[test]
  fn skirt_descends_from_the_edge_into_the_sea() {
    for edge in [-80.0, -5.0, 0.0, 40.0, 900.0] {
      assert_eq!(skirt_height(edge, 0.0, 0.0, 0.7), edge);
      assert!(skirt_height(edge, 0.0, SKIRT_METRES, -1.0) <= -SKIRT_DEPTH);
      assert!(
        skirt_height(edge, 0.0, SKIRT_METRES * 3.0, 1.0)
          < skirt_height(edge, 0.0, SKIRT_METRES, 1.0)
      );

      let mut previous = edge;

      for step in 1..=400 {
        let height = skirt_height(edge, 0.0, step as f32 * 10.0, 0.0);
        assert!(height <= previous, "{edge} at {step}");
        previous = height;
      }
    }

    // Noise moves the skirt by at most 15 % of its drop.
    let drop = 300.0 + SKIRT_DEPTH;
    let quiet = skirt_height(300.0, 0.0, SKIRT_METRES * 0.5, 0.0);
    let noisy = skirt_height(300.0, 0.0, SKIRT_METRES * 0.5, 1.0);
    assert!((noisy - quiet - 0.15 * drop).abs() < 0.01);
  }

  #[test]
  fn skirt_constants_match_the_shader() {
    let source = include_str!("../shaders/common.wgsl");
    assert!(source.contains(&format!("const SKIRT_METRES: f32 = {SKIRT_METRES:.1};")));
    assert!(source.contains(&format!("sea - {SKIRT_DEPTH:.1}")));
    assert!(source.contains(&format!("* {SKIRT_SLOPE}")));
    assert!(source.contains(&format!("xz / {SKIRT_NOISE_METRES:.1}")));
    // The same profile: noise moves the skirt by 15 % of its drop at most.
    assert!(source.contains("(edge - foot) * 0.6 * noise * s * (1.0 - s)"));
  }

  #[test]
  fn skirt_noise_is_smooth_and_in_range() {
    let mut previous = skirt_noise(-5000.0, 1234.0);

    for step in 1..2000 {
      let value = skirt_noise(-5000.0 + step as f32 * 5.0, 1234.0);
      assert!((-1.0..=1.0).contains(&value));
      assert!((value - previous).abs() < 0.05, "{step}");
      previous = value;
    }

    assert_eq!(hash12(3.0, 7.0), hash12(3.0, 7.0));
    assert_ne!(skirt_noise(0.0, 0.0), skirt_noise(350.0, 350.0));
  }

  #[test]
  fn vertices_beyond_the_edge_keep_their_places_on_the_skirt() {
    let metadata = TerrainMetadata {
      metres_per_sample: 10.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(64, 64, 0.0, metadata);

    for (index, height) in map.heights.iter_mut().enumerate() {
      *height = 20.0 + (index % 64) as f32;
    }

    let (normals, surface) = bake_terrain_shading(&map, &BiomeOptions::default(), None);
    let mesh = build_terrain_mesh_centred(&map, &normals, &surface, 5.0, 60.0, 129);
    let half = 31.5 * 10.0;
    let inside = |p: &[f32; 3]| p[0].abs() <= half + 0.01 && p[2].abs() <= half + 0.01;
    let edge: Vec<[f32; 3]> = mesh
      .vertices
      .iter()
      .map(|v| v.position)
      .filter(|p| inside(p) && (p[0].abs() > half - 0.01 || p[2].abs() > half - 0.01))
      .collect();
    let outside: Vec<&TerrainVertex> = mesh
      .vertices
      .iter()
      .filter(|v| !inside(&v.position))
      .collect();

    assert!(!edge.is_empty() && !outside.is_empty());

    for vertex in &outside {
      let p = vertex.position;
      assert!(!edge.iter().any(|e| e[0] == p[0] && e[2] == p[2]), "{p:?}");
      assert!((p[1] - skirt_ground(&map, p[0], p[2])).abs() < 1e-3);
      assert_eq!(vertex.biome[1], 0, "no trees on the skirt");
    }

    // Far out, the skirt is sea floor.
    assert!(outside.iter().any(|v| v.position[1] < -SKIRT_DEPTH));
    // Every vertex within the skirt's reach is distinct: none collapse
    // onto the border, only onto the far line of the skirt.
    let limit = half + SKIRT_MESH_METRES - 0.01;
    let near: Vec<(u32, u32)> = mesh
      .vertices
      .iter()
      .filter(|v| v.position[0].abs() < limit && v.position[2].abs() < limit)
      .map(|v| (v.position[0].to_bits(), v.position[2].to_bits()))
      .collect();
    let mut positions = near.clone();
    positions.sort_unstable();
    positions.dedup();
    assert_eq!(positions.len(), near.len());
    assert!(mesh
      .vertices
      .iter()
      .all(|v| v.position[0].abs() <= limit + 10.0 && v.position[2].abs() <= limit + 10.0));
  }

  #[test]
  fn skirt_ground_matches_the_heights_inside_the_footprint() {
    let metadata = TerrainMetadata {
      metres_per_sample: 5.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(16, 16, 0.0, metadata);

    for (index, height) in map.heights.iter_mut().enumerate() {
      *height = index as f32;
    }

    let (x, z) = (5.0 * 3.0 - 37.5, 5.0 * 4.0 - 37.5);
    assert!((skirt_ground(&map, x, z) - map.heights[4 * 16 + 3]).abs() < 1e-3);
  }

  #[test]
  fn world_to_sample_round_trips_through_build_terrain_mesh() {
    let map = HeightMap::flat(129, 129, 2.0, TerrainMetadata::default());
    let (sample_x, sample_z) = world_to_sample_coordinates(&map, 0.0, 0.0);

    assert!((sample_x - 64.0).abs() < 0.001);
    assert!((sample_z - 64.0).abs() < 0.001);
  }
}
