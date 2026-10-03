//! Map export: the maps the renderer builds, read back at the terrain's
//! own resolution or resampled to another size, row by row.
//!
//! Each map is read from the same per-sample data the renderer draws
//! from (heights, the classified surface, the carved water and its
//! hydrology, the cover and grass textures), so an export matches what
//! is on screen. The engine gathers those inputs (`EngineCore::export_map`);
//! this module turns them into maps.

use crate::render::water::{RiverNetwork, WATER_KIND_LAKE, WATER_KIND_POOL, WATER_KIND_RIVER};
use crate::terrain::HeightMap;

/// The maps `exportMap` can read, in the order of `MAP_KINDS` in
/// `js/src/map-export.ts`, which passes a kind as its index here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MapKind {
  /// Metres above the datum.
  Height,
  /// `BiomeKind` index.
  Biome,
  /// Which water covers a sample ([`WATER_LEGEND`]).
  Water,
  /// Water depth in metres.
  WaterDepth,
  /// Upstream drainage area in square kilometres.
  Flow,
  /// Mean discharge in cubic metres per second.
  Discharge,
  /// Material weights in `MAT_*` order.
  Materials,
  /// Slope in degrees.
  Slope,
  /// World-space unit normals.
  Normals,
  /// Ambient occlusion, 0 occluded to 255 open.
  Occlusion,
  /// Mean annual temperature in °C.
  Temperature,
  /// Moisture, 0 arid to 255 saturated.
  Moisture,
  /// Trees per hectare / 4.
  TreeDensity,
  /// Ground covered by grass, 0 to 255.
  GrassDensity,
  /// Metres above the datum before any carving: the heights as loaded,
  /// without rivers, lakes, the water mask or glacier ice.
  SourceHeight,
}

impl MapKind {
  /// Every kind, by index.
  pub const ALL: [MapKind; 15] = [
    Self::Height,
    Self::Biome,
    Self::Water,
    Self::WaterDepth,
    Self::Flow,
    Self::Discharge,
    Self::Materials,
    Self::Slope,
    Self::Normals,
    Self::Occlusion,
    Self::Temperature,
    Self::Moisture,
    Self::TreeDensity,
    Self::GrassDensity,
    Self::SourceHeight,
  ];

  /// Values per sample.
  pub fn channels(self) -> u32 {
    match self {
      Self::Materials => 12,
      Self::Normals => 3,
      _ => 1,
    }
  }

  /// Whether the map holds `f32` values; the others hold bytes.
  pub fn is_float(self) -> bool {
    matches!(
      self,
      Self::Height
        | Self::SourceHeight
        | Self::WaterDepth
        | Self::Flow
        | Self::Discharge
        | Self::Slope
        | Self::Normals
        | Self::Temperature
    )
  }

  /// How the map is resampled: categories by the nearest sample,
  /// weights and directions bilinearly (then normalised), and every
  /// other map bicubically.
  pub fn filter(self) -> Filter {
    match self {
      Self::Biome | Self::Water => Filter::Nearest,
      Self::Materials | Self::Normals => Filter::Linear,
      _ => Filter::Cubic,
    }
  }
}

/// A resampling filter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Filter {
  /// The nearest sample.
  Nearest,
  /// Bilinear.
  Linear,
  /// Bicubic (Catmull-Rom), which passes through every sample.
  Cubic,
}

/// An exported map's values, row-major with the channels interleaved.
#[derive(Clone, Debug, PartialEq)]
pub enum MapData {
  /// Float maps.
  F32(Vec<f32>),
  /// Byte maps.
  U8(Vec<u8>),
}

/// How to read an exported map's values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapEncoding {
  /// Units of `value x scale`.
  pub units: Option<&'static str>,
  /// A stored value times this is the value in `units`.
  pub scale: Option<f32>,
  /// Lowest and highest value in the map, for float maps.
  pub range: Option<[f32; 2]>,
  /// Name and colour (0 to 1) of each value of a categorical map, or of
  /// each channel of `materials`, in order.
  pub legend: Option<Legend>,
  /// Metres between neighbouring pixels.
  pub metres_per_pixel: [f32; 2],
  /// Sea level in metres.
  pub sea_level_metres: f32,
  /// Where the terrain came from and its generator's version, such as
  /// `vistawasm-fractal-0.2.0`.
  pub generator: String,
}

/// An exported map.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportedMap {
  /// Pixels per row.
  pub width: u32,
  /// Rows.
  pub height: u32,
  /// Values per pixel.
  pub channels: u32,
  /// The values.
  pub data: MapData,
  /// How to read them.
  pub encoding: MapEncoding,
}

/// Values of the `water` map.
pub const WATER_NONE: u8 = 0;
/// See [`WATER_NONE`].
pub const WATER_RIVER: u8 = 1;
/// See [`WATER_NONE`].
pub const WATER_LAKE: u8 = 2;
/// See [`WATER_NONE`].
pub const WATER_OCEAN: u8 = 3;
/// A waterfall, its plunge pool included.
pub const WATER_FALL: u8 = 4;
/// See [`WATER_NONE`].
pub const WATER_PAINTED_RIVER: u8 = 5;
/// See [`WATER_NONE`].
pub const WATER_PAINTED_LAKE: u8 = 6;

/// The `water` map's legend: name and colour of each value.
pub const WATER_LEGEND: [(&str, [f32; 3]); 7] = [
  ("none", [0.0, 0.0, 0.0]),
  ("river", [0.2, 0.55, 0.9]),
  ("lake", [0.1, 0.35, 0.75]),
  ("ocean", [0.05, 0.15, 0.45]),
  ("waterfall", [0.85, 0.95, 1.0]),
  ("paintedRiver", [0.3, 0.8, 0.85]),
  ("paintedLake", [0.15, 0.6, 0.6]),
];

/// The `materials` map's channels, in `MAT_*` order, with a colour for
/// each.
pub const MATERIAL_LEGEND: [(&str, [f32; 3]); 12] = [
  ("lushGrass", [0.3, 0.55, 0.2]),
  ("dryGrass", [0.7, 0.65, 0.35]),
  ("forestFloor", [0.3, 0.25, 0.15]),
  ("sand", [0.85, 0.78, 0.55]),
  ("rock", [0.5, 0.48, 0.45]),
  ("snow", [0.95, 0.96, 0.98]),
  ("mud", [0.35, 0.27, 0.2]),
  ("volcanic", [0.2, 0.17, 0.16]),
  ("ice", [0.75, 0.88, 0.95]),
  ("tundra", [0.55, 0.5, 0.35]),
  ("gravel", [0.6, 0.57, 0.52]),
  ("scree", [0.45, 0.43, 0.4]),
];

/// Numbers per tree in the packed array `exportTrees` returns: x, y, z,
/// species index, variant, scale, rotation, tint, dryness, and 1 for a
/// hand-placed tree or 0.
pub const TREE_RECORD_FLOATS: usize = 10;

/// Name and colour of each value of a categorical map, in order.
pub type Legend = &'static [(&'static str, [f32; 3])];

/// A map's units, the scale from its stored values to them, and its
/// legend.
pub fn describe(kind: MapKind) -> (Option<&'static str>, Option<f32>, Option<Legend>) {
  let unit = 1.0 / 255.0;

  match kind {
    MapKind::Height | MapKind::SourceHeight | MapKind::WaterDepth => (Some("m"), None, None),
    MapKind::Biome => (None, None, Some(&crate::terrain::biomes::BIOME_LEGEND)),
    MapKind::Water => (None, None, Some(&WATER_LEGEND)),
    MapKind::Flow => (Some("km²"), None, None),
    MapKind::Discharge => (Some("m³/s"), None, None),
    MapKind::Materials => (Some("weight"), Some(unit), Some(&MATERIAL_LEGEND)),
    MapKind::Slope => (Some("degrees"), None, None),
    MapKind::Normals => (Some("unit vector"), None, None),
    MapKind::Occlusion => (Some("openness"), Some(unit), None),
    MapKind::Temperature => (Some("°C"), None, None),
    MapKind::Moisture => (Some("saturation"), Some(unit), None),
    MapKind::TreeDensity => (Some("trees/ha"), Some(4.0), None),
    MapKind::GrassDensity => (Some("cover"), Some(unit), None),
  }
}

/// Scale blended material weights to sum to 255 once rounded: the
/// rounding error goes to the largest weight. Weights summing to nothing
/// stay so.
pub fn normalise_weights(weights: &mut [f32]) {
  let sum: f32 = weights.iter().sum();

  if sum <= 0.0 {
    return;
  }

  let (mut total, mut largest, mut most) = (0.0, 0, f32::MIN);
  let scale = 255.0 / sum;

  for (index, weight) in weights.iter_mut().enumerate() {
    // Weights are never negative; `floor` is one instruction in WASM.
    *weight = (*weight * scale + 0.5).floor();
    total += *weight;

    if *weight > most {
      (largest, most) = (index, *weight);
    }
  }

  weights[largest] += 255.0 - total;
}

/// Up to four source indices and their weights, for one output
/// coordinate.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Taps {
  index: [usize; 4],
  weight: [f32; 4],
  count: usize,
}

/// The taps of `filter` at source coordinate `at` on an axis of `size`
/// samples. A whole-number coordinate is one tap of weight 1, so export
/// at the native size copies every value exactly.
fn taps(filter: Filter, at: f64, size: usize) -> Taps {
  let last = size as i64 - 1;
  let clamp = |i: i64| i.clamp(0, last) as usize;
  let base = at.floor();
  let t = (at - base) as f32;
  let i = base as i64;
  let mut taps = Taps {
    index: [clamp(i); 4],
    weight: [1.0, 0.0, 0.0, 0.0],
    count: 1,
  };

  match filter {
    Filter::Nearest => taps.index[0] = clamp(at.round() as i64),
    _ if t == 0.0 => {}
    Filter::Linear => {
      taps.index[1] = clamp(i + 1);
      taps.weight = [1.0 - t, t, 0.0, 0.0];
      taps.count = 2;
    }
    Filter::Cubic => {
      let (t2, t3) = (t * t, t * t * t);
      taps.index = [clamp(i - 1), clamp(i), clamp(i + 1), clamp(i + 2)];
      taps.weight = [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
      ];
      taps.count = 4;
    }
  }

  taps
}

/// The source coordinate of output pixel `i` of `out`, when `size`
/// source samples span the same extent: corners meet corners.
fn source_coordinate(i: usize, size: usize, out: usize) -> f64 {
  if out < 2 {
    return 0.0;
  }

  i as f64 * (size - 1) as f64 / (out - 1) as f64
}

/// Resample a `width x height` map of `channels` values per sample to
/// `out_width x out_height` with `filter`, one output row at a time.
/// `read(index, values)` fills the channels of source sample `index`;
/// `emit(row)` receives each output row, channels interleaved, and may
/// change it.
/// Separable: each source row is filtered across once and kept while
/// the rows below it still need it.
pub fn resample(
  (width, height): (usize, usize),
  channels: usize,
  (out_width, out_height): (usize, usize),
  filter: Filter,
  read: &dyn Fn(usize, &mut [f32]),
  emit: &mut dyn FnMut(&mut [f32]),
) {
  let columns: Vec<Taps> = (0..out_width)
    .map(|x| taps(filter, source_coordinate(x, width, out_width), width))
    .collect();
  let row_length = out_width * channels;
  // The four source rows filtered across most recently, and which rows.
  let mut cached: [(usize, Vec<f32>); 4] =
    std::array::from_fn(|_| (usize::MAX, vec![0.0; row_length]));
  let mut source_row = vec![0.0; width * channels];
  let mut row = vec![0.0; row_length];

  for y in 0..out_height {
    let rows = taps(filter, source_coordinate(y, height, out_height), height);
    row.fill(0.0);

    for k in 0..rows.count {
      let source = rows.index[k];
      let slot = match cached.iter().position(|(at, _)| *at == source) {
        Some(slot) => slot,
        None => {
          // Rows only move down, so the lowest-numbered row is done with.
          let slot = (0..4)
            .filter(|slot| !rows.index[..rows.count].contains(&cached[*slot].0))
            .min_by_key(|slot| cached[*slot].0.wrapping_add(1))
            .unwrap_or(0);
          // Each source value is read once, however many columns use it.
          for (x, values) in source_row.chunks_exact_mut(channels).enumerate() {
            read(source * width + x, values);
          }

          let across = &mut cached[slot].1;
          across.fill(0.0);

          for (x, column) in columns.iter().enumerate() {
            for tap in 0..column.count {
              let at = column.index[tap] * channels;

              for (channel, value) in source_row[at..at + channels].iter().enumerate() {
                across[x * channels + channel] += column.weight[tap] * value;
              }
            }
          }

          cached[slot].0 = source;
          slot
        }
      };

      for (out, value) in row.iter_mut().zip(&cached[slot].1) {
        *out += rows.weight[k] * value;
      }
    }

    emit(&mut row);
  }
}

/// Which water covers each sample ([`WATER_NONE`] to
/// [`WATER_PAINTED_LAKE`]) and its depth in metres, from the water the
/// renderer draws: river ribbons, lakes and plunge pools where their
/// surface is above the ground, streams narrower than a sample along
/// their centrelines, waterfalls from lip to foot, and the sea wherever
/// the ground is below sea level and no other water lies higher.
/// `painted` is the water mask resampled to the terrain, if one is set.
pub fn water_layers(
  map: &HeightMap,
  rivers: &RiverNetwork,
  painted: Option<&[u8]>,
) -> (Vec<u8>, Vec<f32>) {
  let (width, height) = (map.metadata.width as usize, map.metadata.height as usize);
  let count = width * height;
  let mut kind = vec![WATER_NONE; count];
  let mut level = vec![f32::NEG_INFINITY; count];

  if width < 2 || height < 2 || map.heights.len() != count {
    return (kind, vec![0.0; count]);
  }

  let metres = map.metadata.metres_per_sample.max(0.001);
  let half = [(width as f32 - 1.0) * 0.5, (height as f32 - 1.0) * 0.5];
  let sample = |p: [f32; 3]| [p[0] / metres + half[0], p[2] / metres + half[1], p[1]];
  let vertices = &rivers.vertices;

  for triangle in rivers.indices.chunks_exact(3) {
    let [a, b, c] = [0, 1, 2].map(|k| triangle[k] as usize);
    let Some(vertex) = vertices
      .get(a)
      .filter(|_| b < vertices.len() && c < vertices.len())
    else {
      continue;
    };
    let value = match vertex.kind() {
      k if k == WATER_KIND_RIVER => WATER_RIVER,
      k if k == WATER_KIND_LAKE => WATER_LAKE,
      k if k == WATER_KIND_POOL => WATER_FALL,
      _ => continue,
    };
    let [p, q, r] = [a, b, c].map(|i| sample(vertices[i].position));
    fill_triangle(map, [p, q, r], value, &mut kind, &mut level);

    // The first triangle of a ribbon quad joins two rows of the strip:
    // its centreline is drawn even where it is too narrow to cover a
    // sample centre.
    if value == WATER_RIVER && b == a + 1 && c == a + 2 && a + 3 < vertices.len() {
      let s = sample(vertices[a + 3].position);
      let mid = |u: [f32; 3], v: [f32; 3]| {
        [
          (u[0] + v[0]) * 0.5,
          (u[1] + v[1]) * 0.5,
          (u[2] + v[2]) * 0.5,
        ]
      };
      stamp_line(
        map,
        mid(p, q),
        mid(r, s),
        0.0,
        WATER_RIVER,
        &mut kind,
        &mut level,
        false,
      );
    }
  }

  for fall in rivers.falls.iter().filter(|fall| !fall.trickle) {
    let radius = (0.5 * fall.width / metres).max(0.5);
    stamp_line(
      map,
      [fall.lip[0], fall.lip[1], fall.lip_level],
      [fall.foot[0], fall.foot[1], fall.foot_level],
      radius,
      WATER_FALL,
      &mut kind,
      &mut level,
      true,
    );
  }

  let sea = map.metadata.sea_level_metres;
  let mut depth = vec![0.0; count];

  for index in 0..count {
    let ground = map.heights[index];

    if ground < sea && level[index] < sea {
      kind[index] = WATER_OCEAN;
      level[index] = sea;
    }

    let mask = painted
      .and_then(|mask| mask.get(index))
      .copied()
      .unwrap_or(0);
    kind[index] = match (kind[index], mask) {
      (WATER_RIVER, 1..=127) => WATER_PAINTED_RIVER,
      (WATER_LAKE, 128..) => WATER_PAINTED_LAKE,
      (value, _) => value,
    };

    if kind[index] != WATER_NONE {
      depth[index] = (level[index] - ground).max(0.0);
    }
  }

  (kind, depth)
}

/// Mark the samples whose centres lie in a triangle (sample x, sample z,
/// level) and whose ground is below its surface, where it is the highest
/// water there.
fn fill_triangle(
  map: &HeightMap,
  [p, q, r]: [[f32; 3]; 3],
  value: u8,
  kind: &mut [u8],
  level: &mut [f32],
) {
  let width = map.metadata.width as usize;
  let area = (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);

  if area.abs() < 1e-12 {
    return;
  }

  let bound = |axis: usize, size: u32| {
    let low = p[axis].min(q[axis]).min(r[axis]).ceil().max(0.0) as usize;
    let high = p[axis]
      .max(q[axis])
      .max(r[axis])
      .floor()
      .min(size as f32 - 1.0);
    (low, high)
  };
  let ((x0, x1), (z0, z1)) = (bound(0, map.metadata.width), bound(1, map.metadata.height));

  if x1 < 0.0 || z1 < 0.0 {
    return;
  }

  for z in z0..=z1 as usize {
    for x in x0..=x1 as usize {
      let (fx, fz) = (x as f32, z as f32);
      let u = ((q[0] - fx) * (r[1] - fz) - (q[1] - fz) * (r[0] - fx)) / area;
      let v = ((r[0] - fx) * (p[1] - fz) - (r[1] - fz) * (p[0] - fx)) / area;
      let w = 1.0 - u - v;

      if u < -1e-4 || v < -1e-4 || w < -1e-4 {
        continue;
      }

      let surface = u * p[2] + v * q[2] + w * r[2];
      let index = z * width + x;

      if surface > map.heights[index] && surface > level[index] {
        level[index] = surface;
        kind[index] = value;
      }
    }
  }
}

/// Mark the samples within `radius` samples of the segment from `a` to
/// `b` (sample x, sample z, level), with the level along it; `over`
/// marks them whatever water is there, and otherwise only where none
/// lies higher.
#[allow(clippy::too_many_arguments)]
fn stamp_line(
  map: &HeightMap,
  a: [f32; 3],
  b: [f32; 3],
  radius: f32,
  value: u8,
  kind: &mut [u8],
  level: &mut [f32],
  over: bool,
) {
  let (width, height) = (map.metadata.width as i64, map.metadata.height as i64);
  let length = crate::maths::length2(b[0] - a[0], b[1] - a[1]);
  let steps = (length * 2.0).ceil().clamp(1.0, 1e6) as usize;
  let reach = radius.ceil() as i64;
  // Without a radius, the sample nearest each point along the line.
  let limit = if radius > 0.0 { radius } else { f32::INFINITY };

  for step in 0..=steps {
    let t = step as f32 / steps as f32;
    let [x, z, surface] = [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t);

    for dz in -reach..=reach {
      for dx in -reach..=reach {
        let (sx, sz) = (x.round() as i64 + dx, z.round() as i64 + dz);

        if sx < 0
          || sz < 0
          || sx >= width
          || sz >= height
          || crate::maths::length2(sx as f32 - x, sz as f32 - z) > limit
        {
          continue;
        }

        let index = (sz * width + sx) as usize;

        if over || level[index] < surface {
          level[index] = level[index].max(surface);
          kind[index] = value;
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vista_types::BiomeKind;
  use vista_types::TerrainMetadata;

  fn read_from(values: &[f32]) -> impl Fn(usize, &mut [f32]) + '_ {
    |index, out: &mut [f32]| out[0] = values[index]
  }

  fn run(values: &[f32], size: (usize, usize), out: (usize, usize), filter: Filter) -> Vec<f32> {
    let mut rows = Vec::new();
    resample(size, 1, out, filter, &read_from(values), &mut |row| {
      rows.extend_from_slice(row)
    });
    rows
  }

  #[test]
  fn bicubic_resampling_at_the_native_size_is_the_identity() {
    let values: Vec<f32> = (0..35 * 17)
      .map(|i| ((i * 7919) % 1013) as f32 * 0.37 - 120.0)
      .collect();

    for filter in [Filter::Cubic, Filter::Linear, Filter::Nearest] {
      assert_eq!(run(&values, (35, 17), (35, 17), filter), values);
    }
  }

  #[test]
  fn resampling_keeps_corners_and_follows_a_ramp() {
    // A ramp is reproduced exactly by every filter but the nearest (the
    // bicubic one away from the edges, where it repeats the edge sample).
    let values: Vec<f32> = (0..9 * 5)
      .map(|i| (i % 9) as f32 * 2.0 + (i / 9) as f32)
      .collect();

    for filter in [Filter::Cubic, Filter::Linear] {
      let out = run(&values, (9, 5), (33, 17), filter);
      assert_eq!(out.len(), 33 * 17);

      for (index, value) in out.iter().enumerate() {
        let (x, y) = ((index % 33) as f32 * 0.25, (index / 33) as f32 * 0.25);
        let inside = (1.0..=7.0).contains(&x) && (1.0..=3.0).contains(&y);

        if filter == Filter::Linear || inside {
          assert!(
            (value - (x * 2.0 + y)).abs() < 1e-3,
            "{filter:?} {index} {value}"
          );
        }
      }
    }

    let down = run(&values, (9, 5), (2, 2), Filter::Cubic);
    assert_eq!(down, vec![0.0, 16.0, 4.0, 20.0]);
  }

  #[test]
  fn nearest_resampling_never_invents_a_category() {
    let values: Vec<f32> = (0..16 * 16)
      .map(|i| [3.0, 14.0, 7.0][(i * 5 / 7) % 3])
      .collect();

    for out in [(5, 5), (31, 47), (100, 3)] {
      for value in run(&values, (16, 16), out, Filter::Nearest) {
        assert!([3.0, 14.0, 7.0].contains(&value));
      }
    }
  }

  #[test]
  fn many_channels_resample_together() {
    let values: Vec<f32> = (0..4 * 4 * 3).map(|i| i as f32).collect();
    let mut out = Vec::new();
    resample(
      (4, 4),
      3,
      (7, 7),
      Filter::Linear,
      &|index, v: &mut [f32]| v.copy_from_slice(&values[index * 3..index * 3 + 3]),
      &mut |row| out.extend_from_slice(row),
    );
    assert_eq!(out.len(), 7 * 7 * 3);
    // The middle of the first two samples, channel by channel.
    assert_eq!(&out[3..6], &[1.5, 2.5, 3.5]);
  }

  #[test]
  fn the_biome_legend_covers_every_biome_in_order() {
    let legend = crate::terrain::biomes::BIOME_LEGEND;
    assert_eq!(legend.len(), BiomeKind::ALL.len());

    // Named as the options name them: the variant, starting lower case.
    for ((name, _), biome) in legend.iter().zip(BiomeKind::ALL) {
      let debug = format!("{biome:?}");
      assert_eq!(
        format!("{}{}", debug[..1].to_ascii_lowercase(), &debug[1..]),
        *name
      );
    }

    assert_eq!(legend[3].0, "innerForest");
    assert_eq!(legend[18].0, "iceArctic");
  }

  #[test]
  fn the_material_legend_names_every_channel() {
    use crate::terrain::biomes::*;
    assert_eq!(MATERIAL_LEGEND.len(), MATERIAL_COUNT);
    assert_eq!(MATERIAL_LEGEND[MAT_GRAVEL].0, "gravel");
    assert_eq!(MATERIAL_LEGEND[MAT_SCREE].0, "scree");
    assert_eq!(MATERIAL_LEGEND[MAT_TUNDRA].0, "tundra");
  }

  #[test]
  fn blended_weights_sum_to_255() {
    for weights in [
      vec![85.2, 85.2, 85.2, 0.0],
      vec![1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
      vec![254.4, 0.3, 0.3],
      vec![0.0; 12],
    ] {
      let mut weights = weights;
      let empty = weights.iter().all(|weight| *weight == 0.0);
      normalise_weights(&mut weights);
      let sum: f32 = weights.iter().sum();
      assert!(empty || sum == 255.0, "{weights:?}");
      assert!(weights.iter().all(|weight| (0.0..=255.0).contains(weight)));
    }
  }

  #[test]
  fn the_sea_fills_ground_below_sea_level() {
    let metadata = TerrainMetadata {
      width: 8,
      height: 8,
      metres_per_sample: 10.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(8, 8, 5.0, metadata);
    map.heights[9] = -3.0;
    let (kind, depth) = water_layers(&map, &RiverNetwork::default(), None);
    assert_eq!(kind[9], WATER_OCEAN);
    assert_eq!(depth[9], 3.0);
    assert_eq!(kind.iter().filter(|k| **k != WATER_NONE).count(), 1);
    assert_eq!(depth[0], 0.0);
  }
}
