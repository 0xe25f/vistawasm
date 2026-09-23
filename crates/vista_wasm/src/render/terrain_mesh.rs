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

use crate::terrain::biomes::{classify_surface, SurfaceSample};
use crate::terrain::heightmap::HeightMap;
use crate::terrain::normals::generate_normals;
use vista_types::{BiomeOptions, Vec3};

/// One GPU-ready terrain vertex (40 bytes).
///
/// The layout is tightly packed and matches the vertex buffer layout used by
/// `clipmap_render.wgsl`. Position and normal are stored in terrain metres.
/// The eight surface material weights follow the `MAT_*` order in
/// [`crate::terrain::biomes`] and are normalised `u8`s so the whole vertex
/// stays the same size as the previous four-float material layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TerrainVertex {
  /// World position in terrain metres, centred on the terrain origin.
  pub position: [f32; 3],
  /// Unit surface normal.
  pub normal: [f32; 3],
  /// Lush grass, dry grass, forest floor, and sand weights.
  pub materials_a: [u8; 4],
  /// Rock, snow, mud, and volcanic weights.
  pub materials_b: [u8; 4],
  /// Moisture, temperature, volcanic heat, and ambient occlusion.
  pub climate: [u8; 4],
  /// Biome index, tree cover, river flag, and a reserved byte.
  pub biome: [u8; 4],
}

impl TerrainVertex {
  fn new(position: [f32; 3], normal: Vec3, surface: &SurfaceSample) -> Self {
    let m = surface.materials;

    Self {
      position,
      normal,
      materials_a: [m[0], m[1], m[2], m[3]],
      materials_b: [m[4], m[5], m[6], m[7]],
      climate: [
        surface.moisture,
        surface.temperature,
        surface.heat,
        surface.occlusion,
      ],
      biome: [surface.biome, surface.forest, surface.river, 0],
    }
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
  let width = map.metadata.width;
  let height = map.metadata.height;

  if width == 0 || height == 0 || samples_per_side < 2 || surface.len() != normals.len() {
    return TerrainMeshData {
      vertices: Vec::new(),
      indices: Vec::new(),
    };
  }

  let metres_per_sample = map.metadata.metres_per_sample.max(0.001);
  let half_width = (width as f32 - 1.0) * 0.5;
  let half_height = (height as f32 - 1.0) * 0.5;
  let half_span = (samples_per_side / 2) as i64;
  let centre_x = centre_sample_x.round() as i64;
  let centre_z = centre_sample_z.round() as i64;

  let mut vertices = Vec::with_capacity((samples_per_side * samples_per_side) as usize);

  for sz in 0..samples_per_side {
    let grid_dz = sz as i64 - half_span;
    let sample_z = (centre_z + band_sample_offset(grid_dz)).clamp(0, height as i64 - 1) as u32;

    for sx in 0..samples_per_side {
      let grid_dx = sx as i64 - half_span;
      let sample_x = (centre_x + band_sample_offset(grid_dx)).clamp(0, width as i64 - 1) as u32;
      let index = (sample_z * width + sample_x) as usize;
      let world_x = (sample_x as f32 - half_width) * metres_per_sample;
      let world_z = (sample_z as f32 - half_height) * metres_per_sample;
      vertices.push(TerrainVertex::new(
        [world_x, map.heights[index], world_z],
        normals[index],
        &surface[index],
      ));
    }
  }

  let quads_per_side = samples_per_side - 1;
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

  TerrainMeshData { vertices, indices }
}

const _: () = assert!(std::mem::size_of::<TerrainVertex>() == 40);

#[cfg(test)]
mod tests {
  use super::*;
  use vista_types::TerrainMetadata;

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
  fn world_to_sample_round_trips_through_build_terrain_mesh() {
    let map = HeightMap::flat(129, 129, 2.0, TerrainMetadata::default());
    let (sample_x, sample_z) = world_to_sample_coordinates(&map, 0.0, 0.0);

    assert!((sample_x - 64.0).abs() < 0.001);
    assert!((sample_z - 64.0).abs() < 0.001);
  }
}
