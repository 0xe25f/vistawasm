//! Climate-driven biome classification and surface material weights.
//!
//! Every terrain sample is classified into one of the [`BiomeKind`]s from
//! its height, slope, and two seeded, low-frequency climate fields
//! (temperature and moisture), plus a handful of volcanic hotspots placed
//! on the highest peaks. The same pass also produces the ten surface
//! material weights the terrain shader blends between, so ground textures,
//! tree species, and grass colour all agree with each other.
//!
//! Every sample also gets a mean annual temperature in °C. With
//! `meanTemperatureCelsius` set, it is that sea-level mean plus a little
//! climate noise, cooled by 6.5 °C per 1000 m of altitude, and drives every
//! biome, including glaciers and tundra. Without it, the climate works as
//! it always has, and the °C value (15 °C at sea level, just below freezing
//! on the highest summit) is reported and drives the weather, but forms no
//! ice: the highest summits carry the snowy peak biomes instead.
//!
//! Climate noise is evaluated on a coarse grid (it only varies over
//! kilometres) and bilinearly interpolated, which keeps the full-resolution
//! pass cheap even for 4096-sample terrain.

use vista_types::{BiomeKind, BiomeOptions, Vec3};

use crate::maths::{hash_noise, smoothstep, value_noise};
use crate::terrain::heightmap::HeightMap;

/// Lush green grass.
pub const MAT_LUSH_GRASS: usize = 0;
/// Sun-dried, straw-coloured grass.
pub const MAT_DRY_GRASS: usize = 1;
/// Leaf litter, needles, and moss under trees.
pub const MAT_FOREST_FLOOR: usize = 2;
/// Beach and river sand.
pub const MAT_SAND: usize = 3;
/// Bare rock and scree.
pub const MAT_ROCK: usize = 4;
/// Snow and ice.
pub const MAT_SNOW: usize = 5;
/// Wet mud and silt.
pub const MAT_MUD: usize = 6;
/// Basalt and volcanic ash.
pub const MAT_VOLCANIC: usize = 7;
/// Glacier ice.
pub const MAT_ICE: usize = 8;
/// Tundra moss, lichen, and stones.
pub const MAT_TUNDRA: usize = 9;
/// River gravel and cobbles.
pub const MAT_GRAVEL: usize = 10;

/// Number of surface materials.
pub const MATERIAL_COUNT: usize = vista_types::MATERIAL_COUNT;

/// Temperature unit (0 to 1) to °C, and back. The same mapping is used by
/// the surface texture the shaders read.
pub fn unit_to_celsius(unit: f32) -> f32 {
  unit * 65.0 - 30.0
}

/// °C to the 0 to 1 temperature unit, unclamped.
pub fn celsius_to_unit(celsius: f32) -> f32 {
  (celsius + 30.0) / 65.0
}

/// Compact per-sample surface description, 20 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceSample {
  /// Material weights in `MAT_*` order, summing to roughly 255.
  pub materials: [u8; MATERIAL_COUNT],
  /// Moisture from 0 (arid) to 255 (saturated).
  pub moisture: u8,
  /// Temperature from 0 (polar) to 255 (tropical).
  pub temperature: u8,
  /// Volcanic heat from 0 to 255; drives lava glow in calderas.
  pub heat: u8,
  /// Ambient occlusion from 0 (fully occluded) to 255 (open).
  pub occlusion: u8,
  /// [`BiomeKind`] as `u8`.
  pub biome: u8,
  /// Tree cover likelihood from 0 to 255.
  pub forest: u8,
  /// Whether a river runs through this sample (0 or 255).
  pub river: u8,
  /// Snow that never melts, from 0 to 255: 255 on glacier, up to 160 on
  /// tundra. On cold sea next to land it marks snow-covered fast ice.
  pub permanent_snow: u8,
  /// Mean annual temperature in hundredths of a °C.
  pub celsius_hundredths: i16,
}

impl SurfaceSample {
  /// The biome of this sample.
  pub fn biome_kind(&self) -> BiomeKind {
    BiomeKind::from_index(self.biome)
  }

  /// Material weight as a 0 to 1 float.
  pub fn weight(&self, material: usize) -> f32 {
    self.materials.get(material).copied().unwrap_or(0) as f32 / 255.0
  }

  /// Moisture as a 0 to 1 float.
  pub fn moisture_unit(&self) -> f32 {
    self.moisture as f32 / 255.0
  }

  /// Temperature as a 0 to 1 float.
  pub fn temperature_unit(&self) -> f32 {
    self.temperature as f32 / 255.0
  }

  /// Mean annual temperature in °C.
  pub fn celsius(&self) -> f32 {
    self.celsius_hundredths as f32 / 100.0
  }

  /// Permanent snow as a 0 to 1 float.
  pub fn permanent_snow_unit(&self) -> f32 {
    self.permanent_snow as f32 / 255.0
  }

  /// Whether this sample is glacier ice rather than tundra.
  pub fn is_glacier(&self) -> bool {
    self.biome == BiomeKind::IceArctic as u8 && self.permanent_snow == 255
  }

  /// Whether this sample is the tundra fringe of the ice.
  pub fn is_tundra(&self) -> bool {
    self.biome == BiomeKind::IceArctic as u8 && self.permanent_snow < 255
  }
}

/// A volcanic hotspot centred on a high peak.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volcano {
  /// Heightmap sample x.
  pub x: f32,
  /// Heightmap sample y.
  pub y: f32,
  /// Radius of the volcanic region in samples.
  pub radius: f32,
}

/// Maximum coarse climate grid samples per side.
const CLIMATE_GRID: u32 = 129;

/// Fractal value noise in roughly -1 to 1.
fn fbm(seed: u64, x: f32, y: f32, octaves: u32) -> f32 {
  let mut value = 0.0;
  let mut amplitude = 1.0;
  let mut frequency = 1.0;
  let mut total = 0.0;

  for octave in 0..octaves {
    value += value_noise(
      seed.wrapping_add(octave as u64 * 7_919),
      x * frequency,
      y * frequency,
    ) * amplitude;
    total += amplitude;
    amplitude *= 0.5;
    frequency *= 2.03;
  }

  value / total.max(0.0001)
}

/// Coarse climate fields (temperature and moisture before altitude
/// effects), bilinearly sampled at full resolution.
struct ClimateGrid {
  width: u32,
  height: u32,
  step: f32,
  temperature: Vec<f32>,
  moisture: Vec<f32>,
  /// Temperature noise for the °C climate, centred on the map's mean so
  /// `meanTemperatureCelsius` really is the mean. Most of a map lies
  /// within about ±0.5.
  noise: Vec<f32>,
}

impl ClimateGrid {
  fn new(map: &HeightMap, options: &BiomeOptions) -> Self {
    let map_width = map.metadata.width.max(1);
    let map_height = map.metadata.height.max(1);
    let largest = map_width.max(map_height);
    let width = CLIMATE_GRID.min(map_width);
    let height = CLIMATE_GRID.min(map_height);
    let step =
      (largest.saturating_sub(1)) as f32 / (width.max(height).saturating_sub(1)).max(1) as f32;
    let metres_per_sample = map.metadata.metres_per_sample.max(0.001);
    let scale = options.climate_scale_metres.max(100.0);
    let seed = options.seed_offset;
    let mut temperature = Vec::with_capacity((width * height) as usize);
    let mut moisture = Vec::with_capacity((width * height) as usize);
    let mut noise = Vec::with_capacity((width * height) as usize);

    for gy in 0..height {
      for gx in 0..width {
        let world_x = gx as f32 * step * metres_per_sample / scale;
        let world_y = gy as f32 * step * metres_per_sample / scale;
        let warp_x = fbm(seed ^ 0x51f1, world_x * 0.7, world_y * 0.7, 3) * 0.6;
        let warp_y = fbm(seed ^ 0x2c3a, world_x * 0.7 + 5.2, world_y * 0.7 + 1.3, 3) * 0.6;
        let t = fbm(seed ^ 0x7e11, world_x + warp_x, world_y + warp_y, 4);
        let m = fbm(
          seed ^ 0x3d99,
          world_x * 1.3 - warp_y,
          world_y * 1.3 + warp_x,
          4,
        );
        noise.push(t);

        if options.enabled {
          temperature.push(0.56 + t * 0.62 + options.temperature_bias.clamp(-1.0, 1.0) * 0.4);
          moisture.push(0.52 + m * 0.75 + options.moisture_bias.clamp(-1.0, 1.0) * 0.4);
        } else {
          temperature.push(0.46);
          moisture.push(0.5 + m * 0.55);
        }
      }
    }

    let mean = noise.iter().sum::<f32>() / noise.len().max(1) as f32;

    for value in &mut noise {
      *value -= mean;
    }

    Self {
      width,
      height,
      step: step.max(0.0001),
      temperature,
      moisture,
      noise,
    }
  }

  fn sample(&self, x: u32, y: u32) -> (f32, f32, f32) {
    let fx = (x as f32 / self.step).min((self.width - 1) as f32);
    let fy = (y as f32 / self.step).min((self.height - 1) as f32);
    let x0 = fx.floor() as u32;
    let y0 = fy.floor() as u32;
    let x1 = (x0 + 1).min(self.width - 1);
    let y1 = (y0 + 1).min(self.height - 1);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;
    let at = |values: &[f32], gx: u32, gy: u32| values[(gy * self.width + gx) as usize];
    let bilinear = |values: &[f32]| {
      let top = at(values, x0, y0) * (1.0 - tx) + at(values, x1, y0) * tx;
      let bottom = at(values, x0, y1) * (1.0 - tx) + at(values, x1, y1) * tx;
      top * (1.0 - ty) + bottom * ty
    };

    (
      bilinear(&self.temperature),
      bilinear(&self.moisture),
      bilinear(&self.noise),
    )
  }
}

/// Sea-level mean temperature, in °C, of a map without
/// `meanTemperatureCelsius`: a temperate climate.
pub const DEFAULT_SEA_LEVEL_CELSIUS: f32 = 15.0;

/// Cooling from sea level to the highest summit of a map without
/// `meanTemperatureCelsius`, in °C. The summit ends up a little below
/// freezing, where the snowy peak biomes are.
const DEFAULT_RELIEF_COOLING_CELSIUS: f32 = 19.0;

/// Real atmospheric lapse rate, in °C per metre.
const LAPSE_RATE_PER_METRE: f32 = 0.0065;

/// Climate noise, in °C per unit of centred noise: most of a map lies
/// within ±4 °C of the mean, and no sample more than 6 °C from it.
const CLIMATE_NOISE_CELSIUS: f32 = 8.0;

/// The sea-level mean temperature in °C before climate noise, including
/// `temperatureBias`.
pub fn sea_level_celsius(options: &BiomeOptions) -> f32 {
  let bias = if options.enabled {
    options.temperature_bias.clamp(-1.0, 1.0) * 0.4 * 65.0
  } else {
    0.0
  };

  options
    .mean_temperature_celsius
    .unwrap_or(DEFAULT_SEA_LEVEL_CELSIUS)
    + bias
}

/// Local mean temperature in °C. `above_sea` is the altitude in metres,
/// `rel` the same altitude as a fraction of the map's relief, and `relief`
/// the height of the highest point above sea level.
fn local_celsius(options: &BiomeOptions, noise: f32, above_sea: f32, rel: f32, relief: f32) -> f32 {
  let sea_level = sea_level_celsius(options) + (noise * CLIMATE_NOISE_CELSIUS).clamp(-6.0, 6.0);

  if options.mean_temperature_celsius.is_some() {
    sea_level - above_sea.max(0.0) * LAPSE_RATE_PER_METRE
  } else {
    // Low islands and hills are not cooled all the way: they are not
    // alpine.
    let hills = smoothstep((relief - 150.0) / 450.0);
    sea_level - rel.clamp(0.0, 1.0) * DEFAULT_RELIEF_COOLING_CELSIUS * hills
  }
}

/// How the cold shapes a sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColdGround {
  /// Too warm for ice or tundra.
  None,
  /// Glacier ice.
  Glacier,
  /// Tundra: moss and lichen where the ice thins.
  Tundra,
  /// Cold enough for ice but too steep to hold it: rock with snow.
  Cliff,
}

/// The mean temperature, in °C, below which ice builds up into a glacier.
/// Glaciers need snowfall as well as cold: -2 °C is enough in the wettest
/// climates, average ones need about -6 °C, and dry ones stay bare tundra
/// and polar desert down to about -16 °C. `moisture` is the large-scale
/// climate moisture, 0 to 1.
fn glacier_celsius(moisture: f32) -> f32 {
  -2.0 - (0.7 - moisture).max(0.0) * 20.0
}

/// The ice and tundra rules. Ice flows off slopes steeper than about 35
/// degrees unless it is 4 °C colder still, and nothing holds above 50
/// degrees.
fn cold_ground(celsius: f32, slope_degrees: f32, moisture: f32) -> ColdGround {
  let glacier = glacier_celsius(moisture);

  if celsius < glacier {
    if slope_degrees < 35.0 || (celsius < glacier - 4.0 && slope_degrees < 50.0) {
      ColdGround::Glacier
    } else if slope_degrees < 50.0 {
      ColdGround::Tundra
    } else {
      ColdGround::Cliff
    }
  } else if celsius < 3.0 && slope_degrees < 50.0 {
    ColdGround::Tundra
  } else if celsius < -2.0 && slope_degrees >= 50.0 {
    ColdGround::Cliff
  } else {
    ColdGround::None
  }
}

/// How strongly the cold may act here, from 0 to 1. Only a climate
/// temperature brings ice and tundra: without one, the highest summits
/// carry the snowy peak biomes instead.
fn cold_gate(options: &BiomeOptions) -> f32 {
  if options.enabled && options.mean_temperature_celsius.is_some() {
    1.0
  } else {
    0.0
  }
}

fn slope_degrees(normal: Vec3) -> f32 {
  normal[1].clamp(-1.0, 1.0).acos().to_degrees()
}

/// Large-scale moisture before local detail: the climate field, wetter in
/// the lowlands.
fn climate_moisture(base_moisture: f32, rel: f32) -> f32 {
  (base_moisture + (1.0 - rel).powi(3) * 0.12).clamp(0.0, 1.0)
}

/// Sea level and the relief used for relative altitude.
fn relief(map: &HeightMap) -> (f32, f32) {
  let sea = map.metadata.sea_level_metres;
  (sea, (map.metadata.max_height_metres - sea).max(50.0))
}

/// Mark the samples that hold glacier ice, without classifying the rest
/// of the surface. Used to shape glaciers before the full classification.
pub fn glacier_mask(map: &HeightMap, normals: &[Vec3], options: &BiomeOptions) -> Vec<bool> {
  let width = map.metadata.width;
  let height = map.metadata.height;
  let count = (width as usize) * (height as usize);

  if count == 0 || normals.len() != count || !options.enabled {
    return vec![false; count];
  }

  let (sea, range) = relief(map);
  let climate = ClimateGrid::new(map, options);
  let mut mask = vec![false; count];

  for y in 0..height {
    for x in 0..width {
      let index = (y * width + x) as usize;
      let h = map.heights[index];
      let rel = ((h - sea) / range).clamp(0.0, 1.0);

      let slope = slope_degrees(normals[index]);

      if map.no_data[index] || h < sea - 0.3 || cold_gate(options) <= 0.0 {
        continue;
      }

      let (_, base_moisture, noise) = climate.sample(x, y);
      let celsius = local_celsius(options, noise, h - sea, rel, range);
      let moisture = climate_moisture(base_moisture, rel);
      mask[index] = cold_ground(celsius, slope, moisture) == ColdGround::Glacier;
    }
  }

  mask
}

/// Find volcanic hotspots on the highest, well-separated peaks.
pub fn find_volcanoes(map: &HeightMap, options: &BiomeOptions) -> Vec<Volcano> {
  let volcanism = options.volcanism.clamp(0.0, 1.0);

  if !options.enabled || volcanism <= 0.05 {
    return Vec::new();
  }

  let width = map.metadata.width;
  let height = map.metadata.height;

  if width < 16 || height < 16 {
    return Vec::new();
  }

  let sea = map.metadata.sea_level_metres;
  let range = (map.metadata.max_height_metres - sea).max(1.0);
  let cells = 8;
  let cell_width = width / cells;
  let cell_height = height / cells;
  let stride = (cell_width.min(cell_height) / 16).max(1);
  let mut peaks: Vec<(f32, u32, u32)> = Vec::new();

  for cy in 0..cells {
    for cx in 0..cells {
      let mut best: Option<(f32, u32, u32)> = None;
      let mut y = cy * cell_height;

      while y < ((cy + 1) * cell_height).min(height) {
        let mut x = cx * cell_width;

        while x < ((cx + 1) * cell_width).min(width) {
          let index = (y * width + x) as usize;

          if !map.no_data[index] {
            let h = map.heights[index];

            if best.is_none_or(|(bh, _, _)| h > bh) {
              best = Some((h, x, y));
            }
          }

          x += stride;
        }

        y += stride;
      }

      if let Some(peak) = best {
        if (peak.0 - sea) / range > 0.5 {
          peaks.push(peak);
        }
      }
    }
  }

  peaks.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

  let wanted = (volcanism * 3.0).round().max(1.0) as usize;
  let radius = width.max(height) as f32 * (0.05 + 0.07 * volcanism);
  let mut volcanoes: Vec<Volcano> = Vec::new();

  for (index, (_, x, y)) in peaks.iter().enumerate() {
    if volcanoes.len() >= wanted {
      break;
    }

    // Skip some candidates by seed so different seeds pick different
    // peaks, but never skip the very last viable ones.
    let skip_roll = (hash_noise(options.seed_offset ^ 0xa11c, *x as i32, *y as i32) + 1.0) * 0.5;

    if skip_roll < 0.35 && peaks.len() - index > wanted - volcanoes.len() {
      continue;
    }

    let separated = volcanoes.iter().all(|volcano| {
      let dx = volcano.x - *x as f32;
      let dy = volcano.y - *y as f32;
      (dx * dx + dy * dy).sqrt() > radius * 2.2
    });

    if separated {
      volcanoes.push(Volcano {
        x: *x as f32,
        y: *y as f32,
        radius,
      });
    }
  }

  volcanoes
}

/// The automatic snow line never sits lower than this above sea level, so
/// low hills do not whiten just for being the highest ground on the map.
pub const MIN_AUTOMATIC_SNOW_LINE_METRES: f32 = 400.0;

/// Depth of the alpine transition band below the snow line, as a fraction
/// of the relief (clamped to 60 to 400 m).
const TRANSITION_BAND_FRACTION: f32 = 0.15;

/// The upper snowy peaks start this fraction of the way from the snow
/// line to the highest peak.
const UPPER_SNOW_FRACTION: f32 = 0.5;

/// Resolve the automatic snow line for a heightmap: 80 % of the way from
/// sea level to the highest peak, and at least
/// [`MIN_AUTOMATIC_SNOW_LINE_METRES`] above sea level.
pub fn snow_line_metres(map: &HeightMap, options: &BiomeOptions) -> f32 {
  let sea = map.metadata.sea_level_metres;
  let relative = (map.metadata.max_height_metres - sea).max(1.0) * 0.8;

  options
    .snow_line_metres
    .unwrap_or(sea + relative.max(MIN_AUTOMATIC_SNOW_LINE_METRES))
}

fn classify_into(
  map: &HeightMap,
  normals: &[Vec3],
  river_mask: Option<&[bool]>,
  options: &BiomeOptions,
  only: Option<&[usize]>,
  samples: &mut Vec<SurfaceSample>,
) {
  let width = map.metadata.width;
  let height = map.metadata.height;
  let count = (width as usize) * (height as usize);

  if count == 0 || normals.len() != count {
    samples.clear();
    samples.resize(count, SurfaceSample::default());
    return;
  }

  let (sea, range) = relief(map);
  let climate = ClimateGrid::new(map, options);
  let volcanoes = find_volcanoes(map, options);
  let snow_line = snow_line_metres(map, options);
  let peak = map.metadata.max_height_metres;
  // The transition band scales with the relief: tens of metres on low
  // ranges, a few hundred on high ones.
  let transition_band = (range * TRANSITION_BAND_FRACTION).clamp(60.0, 400.0);
  let beach = options.beach_height_metres.max(0.5);
  let metres_per_sample = map.metadata.metres_per_sample.max(0.001);
  let detail_seed = options.seed_offset ^ 0x0f0f_1234;
  let classify = |x: u32, y: u32| -> SurfaceSample {
    let index = (y * width + x) as usize;
    let h = map.heights[index];
    let normal = normals[index];
    let steep = (1.0 - normal[1]).clamp(0.0, 1.0);
    let rel = ((h - sea) / range).clamp(0.0, 1.0);
    let above_sea = h - sea;
    let (base_temperature, base_moisture, noise) = climate.sample(x, y);
    let lowland = (1.0 - rel).powi(3);
    // The sea surface is at sea level, whatever the depth of the bed.
    let celsius = local_celsius(options, noise, above_sea, rel, range);
    let temperature = if options.enabled && options.mean_temperature_celsius.is_some() {
      celsius_to_unit(celsius).clamp(0.0, 1.0)
    } else {
      (base_temperature - rel * 0.62).clamp(0.0, 1.0)
    };
    let slope = slope_degrees(normal);
    let gate = cold_gate(options);
    let climate_wetness = climate_moisture(base_moisture, rel);
    let cold = if h < sea - 0.3 || gate <= 0.0 {
      ColdGround::None
    } else {
      cold_ground(celsius, slope, climate_wetness)
    };
    // A little high-frequency jitter keeps biome borders organic rather
    // than following the smooth climate contours exactly.
    let jitter = hash_noise(detail_seed, x as i32 / 3, y as i32 / 3) * 0.035
      + value_noise(detail_seed, x as f32 * 0.09, y as f32 * 0.09) * 0.05;
    let is_river = river_mask.is_some_and(|mask| mask.get(index).copied().unwrap_or(false));
    let moisture =
      (base_moisture + lowland * 0.12 + jitter + if is_river { 0.1 } else { 0.0 }).clamp(0.0, 1.0);

    // Cavity occlusion from the four-neighbour Laplacian.
    let neighbour = |dx: i32, dy: i32| {
      let nx = (x as i32 + dx).clamp(0, width as i32 - 1) as u32;
      let ny = (y as i32 + dy).clamp(0, height as i32 - 1) as u32;
      map.heights[(ny * width + nx) as usize]
    };
    let laplacian =
      (neighbour(-2, 0) + neighbour(2, 0) + neighbour(0, -2) + neighbour(0, 2)) * 0.25 - h;
    let occlusion = 1.0 - (laplacian / (metres_per_sample * 1.2)).clamp(0.0, 1.0) * 0.55;

    // Volcanic proximity.
    let mut volcano_factor: f32 = 0.0;
    let mut caldera_factor: f32 = 0.0;

    for volcano in &volcanoes {
      let dx = x as f32 - volcano.x;
      let dy = y as f32 - volcano.y;
      let distance = (dx * dx + dy * dy).sqrt() / volcano.radius;
      volcano_factor = volcano_factor.max(1.0 - smoothstep((distance - 0.55) / 0.45));
      caldera_factor = caldera_factor.max(1.0 - smoothstep((distance - 0.12) / 0.14));
    }

    volcano_factor *= smoothstep((rel - 0.08) / 0.2);
    caldera_factor *= smoothstep((rel - 0.35) / 0.2);

    // The snow line is lower where it is cold, and tropical peaks carry
    // no snow at all.
    let snow_line_here = snow_line - (0.35 - temperature).max(0.0) * range * 0.5;
    let snowy = temperature <= 0.75;
    // The top part of the ground above the snow line is permanent snow
    // and ice; the band below it is snowfield broken by rock.
    let upper_snow_line =
      snow_line_here + ((peak - snow_line_here) * UPPER_SNOW_FRACTION).max(transition_band);

    // Biome decision.
    let biome = if h < sea - 0.3 {
      BiomeKind::Ocean
    } else if caldera_factor > 0.5 {
      BiomeKind::CalderaVolcanic
    } else if volcano_factor > 0.45 {
      BiomeKind::OuterVolcanic
    } else if matches!(cold, ColdGround::Glacier | ColdGround::Tundra) {
      BiomeKind::IceArctic
    } else if cold == ColdGround::Cliff {
      // Cold rock too steep for ice keeps the mountain bands.
      if snowy && h >= upper_snow_line {
        BiomeKind::UpperSnowyPeaks
      } else if snowy && h >= snow_line_here {
        BiomeKind::LowerSnowyPeaks
      } else {
        BiomeKind::MountainProper
      }
    } else if above_sea < beach * 1.6 && steep > 0.22 {
      BiomeKind::CoastalRocky
    } else if above_sea < beach * 1.6 && !(moisture > 0.72 && temperature > 0.45 && steep < 0.05) {
      BiomeKind::CoastalBeach
    } else if above_sea < beach * 5.0 && steep > 0.38 {
      BiomeKind::CoastalRocky
    } else if snowy && h >= upper_snow_line {
      BiomeKind::UpperSnowyPeaks
    } else if snowy && h >= snow_line_here {
      BiomeKind::LowerSnowyPeaks
    } else if snowy && h >= snow_line_here - transition_band {
      BiomeKind::AlpineTransition
    } else if rel > 0.62 || (rel > 0.46 && steep > 0.34) {
      BiomeKind::MountainProper
    } else if rel > 0.4 {
      BiomeKind::MountainFoothills
    } else if moisture > 0.68 && steep < 0.07 && rel < 0.16 && temperature > 0.3 {
      BiomeKind::SwampWetlands
    } else if temperature > 0.64 {
      if moisture > 0.74 {
        BiomeKind::InnerJungle
      } else if moisture > 0.58 {
        BiomeKind::OuterJungle
      } else {
        BiomeKind::SavannahExpanse
      }
    } else if moisture < 0.4 {
      BiomeKind::GrassyMeadows
    } else if moisture < 0.5 {
      BiomeKind::OuterThicket
    } else if moisture < 0.63 {
      BiomeKind::OuterForest
    } else {
      BiomeKind::InnerForest
    };

    // Continuous material fields, so textures blend smoothly across
    // biome borders instead of switching abruptly. Snow lies in patches
    // through the transition band (in hollows first, where drifts
    // collect), covers the lower peaks except on steep rock, and on the
    // upper peaks clings to all but near-vertical faces.
    let into_band = ((h - (snow_line_here - transition_band)) / transition_band).clamp(0.0, 1.0);
    let patches =
      smoothstep((into_band * 1.3 - 0.35 + (1.0 - occlusion) * 0.8 + jitter * 3.0) / 0.35);
    let upper = smoothstep((h - upper_snow_line) / transition_band.max(1.0) + 0.5);
    let steep_limit = 0.3 + upper * 0.25;
    let mut snow = if snowy {
      let settled = smoothstep((h - snow_line_here) / 60.0 + 0.5);
      (settled.max(patches * 0.6 * into_band) * (1.0 - smoothstep((steep - steep_limit) / 0.25)))
        .max(upper * 0.9)
        .min(1.0)
    } else {
      0.0
    };

    // Wherever it is below freezing all year, snow lies on anything flat
    // enough to hold it, whatever the height. Cold, dry snow clings to
    // steeper faces than wet snow does.
    let cold_snow = if h >= sea - 0.3 {
      gate * smoothstep((-2.0 - celsius) / 6.0) * (1.0 - smoothstep((steep - 0.4) / 0.25))
    } else {
      0.0
    };
    snow = snow.max(cold_snow);
    let mountain = smoothstep((rel - 0.5) / 0.25);
    let mut rock = smoothstep((steep - 0.16) / 0.26) * 0.95 + mountain * 0.35 * (1.0 - snow);

    if biome == BiomeKind::CoastalRocky {
      rock = rock.max(0.75);
    }

    // Above the trees the ground is scree and thin turf, stonier the
    // closer it is to the snow.
    if biome == BiomeKind::AlpineTransition {
      rock = rock.max(0.3 + 0.35 * into_band);
    }

    // Snow that never melts buries all but the steepest rock.
    rock *= 1.0 - cold_snow * 0.6;

    let sand_band = 1.0 - smoothstep((above_sea - beach * 0.7) / (beach * 0.9).max(0.5));
    let desert =
      smoothstep((temperature - 0.7) / 0.15) * (1.0 - smoothstep((moisture - 0.2) / 0.15));
    let mut sand = (sand_band * (1.0 - smoothstep((steep - 0.2) / 0.15))).max(desert * 0.6);
    let swamp = if biome == BiomeKind::SwampWetlands {
      0.65
    } else {
      smoothstep((moisture - 0.7) / 0.15) * lowland * 0.4
    };
    let mut mud = swamp;

    if h < sea - 0.3 {
      // Sea bed: sand in the shallows, silt in the deep.
      let depth = sea - h;
      sand = 1.0 - smoothstep((depth - 6.0) / 20.0);
      mud = 1.0 - sand;
      rock *= 0.6;
    }

    let volcanic = volcano_factor.max(caldera_factor);
    let forest_floor = smoothstep((moisture - 0.45) / 0.22) * (1.0 - mountain * 0.6);
    let dryness =
      smoothstep((temperature - 0.5) / 0.25) * (1.0 - smoothstep((moisture - 0.35) / 0.3));
    let dry_grass = dryness.max(if biome == BiomeKind::SavannahExpanse {
      0.7
    } else {
      0.0
    });
    let lush = (1.0 - dry_grass).max(0.0);

    let mut weights = [0.0f32; MATERIAL_COUNT];
    let cover = (1.0 - rock - snow - sand - mud - volcanic).max(0.0);
    let ground_total = (lush * (1.0 - forest_floor) + dry_grass + forest_floor).max(0.0001);
    weights[MAT_LUSH_GRASS] = cover * lush * (1.0 - forest_floor) / ground_total;
    weights[MAT_DRY_GRASS] = cover * dry_grass / ground_total;
    weights[MAT_FOREST_FLOOR] = cover * forest_floor / ground_total;
    weights[MAT_SAND] = sand;
    weights[MAT_ROCK] = rock;
    weights[MAT_SNOW] = snow;
    weights[MAT_MUD] = mud;
    weights[MAT_VOLCANIC] = volcanic;

    if gate > 0.0 && h >= sea - 0.3 {
      apply_cold_materials(
        &mut weights,
        ColdMaterials {
          gate,
          celsius,
          slope,
          glacier: cold == ColdGround::Glacier && biome == BiomeKind::IceArctic,
          glacier_celsius: glacier_celsius(climate_wetness),
          detail: value_noise(detail_seed ^ 0x1ce, x as f32 * 0.07, y as f32 * 0.07),
        },
      );
    }

    let total: f32 = weights.iter().sum::<f32>().max(0.0001);
    let mut materials = [0u8; MATERIAL_COUNT];

    for (slot, weight) in materials.iter_mut().zip(weights.iter()) {
      *slot = ((weight / total) * 255.0).round().clamp(0.0, 255.0) as u8;
    }

    let glacier = biome == BiomeKind::IceArctic && cold == ColdGround::Glacier;
    let forest = if glacier {
      0.0
    } else {
      forest_density(biome, moisture) * (1.0 - rock.min(1.0)) * (1.0 - snow.min(1.0))
    };
    let permanent_snow = if glacier {
      1.0
    } else if biome == BiomeKind::IceArctic {
      // Tundra keeps patches of old snow the colder it gets, up to 160.
      smoothstep((3.0 - celsius) / 5.0) * (160.0 / 255.0)
    } else {
      0.0
    };

    SurfaceSample {
      materials,
      moisture: unit_to_byte(moisture),
      temperature: unit_to_byte(temperature),
      heat: unit_to_byte(caldera_factor * caldera_factor),
      occlusion: unit_to_byte(occlusion),
      biome: biome as u8,
      forest: unit_to_byte(forest),
      river: if is_river { 255 } else { 0 },
      permanent_snow: unit_to_byte(permanent_snow),
      celsius_hundredths: (celsius * 100.0).round().clamp(-32_000.0, 32_000.0) as i16,
    }
  };

  match only {
    Some(indices) if samples.len() == count => {
      for index in indices {
        samples[*index] = classify(*index as u32 % width, *index as u32 / width);
      }

      // Fast ice depends on the distance to land, which may have changed
      // anywhere, so it is marked again from scratch below.
      let ocean = BiomeKind::Ocean as u8;

      for sample in samples.iter_mut() {
        if sample.biome == ocean {
          sample.permanent_snow = 0;
        }
      }
    }
    _ => {
      samples.clear();
      samples.reserve(count);

      for y in 0..height {
        for x in 0..width {
          samples.push(classify(x, y));
        }
      }
    }
  }

  mark_fast_ice(map, samples);
}

/// Classify every heightmap sample into a biome and surface materials.
///
/// `normals` must be the per-sample normals for `map`. `river_mask`, when
/// present, marks samples that sit under a river channel.
pub fn classify_surface(
  map: &HeightMap,
  normals: &[Vec3],
  river_mask: Option<&[bool]>,
  options: &BiomeOptions,
) -> Vec<SurfaceSample> {
  let mut samples = Vec::new();
  classify_into(map, normals, river_mask, options, None, &mut samples);
  samples
}

/// Classify only the samples at `indices` again, after their heights (or
/// those of their neighbours) or their river flag changed. `samples` must
/// hold a full classification of the map with the same relief; the
/// result is the same as classifying the whole map again.
pub fn reclassify_surface(
  map: &HeightMap,
  normals: &[Vec3],
  river_mask: Option<&[bool]>,
  options: &BiomeOptions,
  samples: &mut Vec<SurfaceSample>,
  indices: &[usize],
) {
  classify_into(map, normals, river_mask, options, Some(indices), samples);
}

/// Blend river bed materials (gravel, sand and mud weights for listed
/// samples, from `render::water::bed_materials`) into classified
/// samples, scaling the ground already there by the share they take.
pub fn apply_bed_materials(samples: &mut [SurfaceSample], bed: &[(u32, [u8; 3])]) {
  for (index, weights) in bed {
    let Some(sample) = samples.get_mut(*index as usize) else {
      continue;
    };
    let share: u32 = weights.iter().map(|weight| u32::from(*weight)).sum();
    let keep = 255 - share.min(255);

    for slot in sample.materials.iter_mut() {
      *slot = ((u32::from(*slot) * keep + 127) / 255) as u8;
    }

    for (material, weight) in [MAT_GRAVEL, MAT_SAND, MAT_MUD].into_iter().zip(weights) {
      sample.materials[material] = sample.materials[material].saturating_add(*weight);
    }
  }
}

/// Inputs for [`apply_cold_materials`].
struct ColdMaterials {
  gate: f32,
  celsius: f32,
  /// Where glacier ice starts, from [`glacier_celsius`].
  glacier_celsius: f32,
  slope: f32,
  glacier: bool,
  /// Value noise in -1 to 1 that breaks up snow and stone patches.
  detail: f32,
}

/// Blend glacier ice and tundra into the material weights. Soft edges on
/// temperature and slope keep the borders natural; `cold.glacier` makes
/// sure every sample classified as glacier is mostly ice or snow.
fn apply_cold_materials(weights: &mut [f32; MATERIAL_COUNT], cold: ColdMaterials) {
  let slope_limit = 35.0 + 15.0 * smoothstep((cold.glacier_celsius - 2.0 - cold.celsius) / 4.0);
  let mut ice = smoothstep((cold.glacier_celsius + 1.0 - cold.celsius) / 2.0)
    * (1.0 - smoothstep((cold.slope - slope_limit + 3.0) / 6.0))
    * cold.gate;

  if cold.glacier {
    ice = ice.max(0.85);
  }

  let tundra = (1.0 - smoothstep((cold.celsius - 3.0) / 2.5))
    * (1.0 - smoothstep((cold.slope - 47.0) / 6.0))
    * cold.gate
    * (1.0 - ice);

  if tundra > 0.0 {
    // Moss and lichen replace grass and forest litter; stones break
    // through where the ground is poor.
    let cover = weights[MAT_LUSH_GRASS] + weights[MAT_DRY_GRASS] + weights[MAT_FOREST_FLOOR];

    for slot in [MAT_LUSH_GRASS, MAT_DRY_GRASS, MAT_FOREST_FLOOR] {
      weights[slot] *= 1.0 - tundra;
    }

    let stony = smoothstep((cold.detail - 0.1) / 0.5) * 0.35;
    weights[MAT_TUNDRA] = cover * tundra * (1.0 - stony);
    weights[MAT_ROCK] += cover * tundra * stony;
  }

  if ice > 0.0 {
    for weight in weights.iter_mut() {
      *weight *= 1.0 - ice;
    }

    // Fresh snow lies on the ice: deeper where it is colder and flatter,
    // scoured to bare blue ice in patches and on steeper ice falls.
    let depth = (0.35 + 0.4 * smoothstep((-4.0 - cold.celsius) / 12.0) + cold.detail * 0.25)
      * (1.0 - smoothstep((cold.slope - 8.0) / 20.0) * 0.6);
    let depth = depth.clamp(0.1, 0.85);
    weights[MAT_SNOW] += ice * depth;
    weights[MAT_ICE] = ice * (1.0 - depth);
  }
}

/// Two-pass chamfer distance transform. `distance` holds 0 at the seeds
/// and `f32::MAX` elsewhere on entry, and the distance in metres to the
/// nearest seed on return; `step` is the spacing between samples.
pub fn chamfer_distance(width: usize, height: usize, step: f32, distance: &mut [f32]) {
  let metres = step;
  let diagonal = step * std::f32::consts::SQRT_2;

  for y in 0..height {
    for x in 0..width {
      let index = y * width + x;
      let mut best = distance[index];

      if x > 0 {
        best = best.min(distance[index - 1] + metres);
      }

      if y > 0 {
        best = best.min(distance[index - width] + metres);

        if x > 0 {
          best = best.min(distance[index - width - 1] + diagonal);
        }

        if x + 1 < width {
          best = best.min(distance[index - width + 1] + diagonal);
        }
      }

      distance[index] = best;
    }
  }

  for y in (0..height).rev() {
    for x in (0..width).rev() {
      let index = y * width + x;
      let mut best = distance[index];

      if x + 1 < width {
        best = best.min(distance[index + 1] + metres);
      }

      if y + 1 < height {
        best = best.min(distance[index + width] + metres);

        if x + 1 < width {
          best = best.min(distance[index + width + 1] + diagonal);
        }

        if x > 0 {
          best = best.min(distance[index + width - 1] + diagonal);
        }
      }

      distance[index] = best;
    }
  }
}

/// Fast ice: on sea colder than -10 °C, within 200 m of land, the sea ice
/// is frozen to the shore and snow-covered. Marked with permanent snow on
/// the ocean samples so the water shader can draw it.
fn mark_fast_ice(map: &HeightMap, samples: &mut [SurfaceSample]) {
  let width = map.metadata.width as usize;
  let height = map.metadata.height as usize;
  let ocean = BiomeKind::Ocean as u8;

  if width == 0
    || samples.len() != width * height
    || !samples
      .iter()
      .any(|sample| sample.biome == ocean && sample.celsius() < -10.0)
  {
    return;
  }

  // Distance to land, in metres.
  let mut distance: Vec<f32> = samples
    .iter()
    .map(|sample| if sample.biome == ocean { f32::MAX } else { 0.0 })
    .collect();
  chamfer_distance(
    width,
    height,
    map.metadata.metres_per_sample.max(0.001),
    &mut distance,
  );

  for (sample, distance) in samples.iter_mut().zip(distance) {
    if sample.biome == ocean && sample.celsius() < -10.0 && distance <= 200.0 {
      sample.permanent_snow = 255;
    }
  }
}

/// Base tree cover likelihood for a biome.
pub fn forest_density(biome: BiomeKind, moisture: f32) -> f32 {
  let base = match biome {
    BiomeKind::GrassyMeadows => 0.08,
    BiomeKind::OuterThicket => 0.4,
    BiomeKind::OuterForest => 0.65,
    BiomeKind::InnerForest => 0.95,
    BiomeKind::MountainFoothills => 0.4,
    BiomeKind::MountainProper => 0.1,
    BiomeKind::OuterVolcanic => 0.05,
    BiomeKind::CalderaVolcanic => 0.0,
    BiomeKind::SavannahExpanse => 0.06,
    BiomeKind::CoastalBeach => 0.18,
    BiomeKind::CoastalRocky => 0.1,
    BiomeKind::OuterJungle => 0.7,
    BiomeKind::InnerJungle => 1.0,
    BiomeKind::SwampWetlands => 0.55,
    BiomeKind::Ocean => 0.0,
    BiomeKind::AlpineTransition => 0.06,
    BiomeKind::LowerSnowyPeaks => 0.0,
    BiomeKind::UpperSnowyPeaks => 0.0,
    // Dwarf shrubs on the tundra, at a tenth of a forest's density.
    BiomeKind::IceArctic => 0.1,
  };

  (base * (0.75 + moisture * 0.5)).clamp(0.0, 1.0)
}

fn unit_to_byte(value: f32) -> u8 {
  (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A display colour for each biome, used by the `biomes` debug view and
/// the minimap helpers.
pub fn biome_debug_colour(biome: BiomeKind) -> [f32; 3] {
  match biome {
    BiomeKind::GrassyMeadows => [0.55, 0.8, 0.3],
    BiomeKind::OuterThicket => [0.4, 0.62, 0.22],
    BiomeKind::OuterForest => [0.2, 0.5, 0.18],
    BiomeKind::InnerForest => [0.08, 0.32, 0.1],
    BiomeKind::MountainFoothills => [0.6, 0.55, 0.4],
    BiomeKind::MountainProper => [0.55, 0.55, 0.58],
    BiomeKind::OuterVolcanic => [0.35, 0.2, 0.18],
    BiomeKind::CalderaVolcanic => [0.9, 0.25, 0.05],
    BiomeKind::SavannahExpanse => [0.85, 0.72, 0.35],
    BiomeKind::CoastalBeach => [0.95, 0.88, 0.62],
    BiomeKind::CoastalRocky => [0.5, 0.45, 0.42],
    BiomeKind::OuterJungle => [0.15, 0.7, 0.35],
    BiomeKind::InnerJungle => [0.02, 0.45, 0.2],
    BiomeKind::SwampWetlands => [0.3, 0.38, 0.25],
    BiomeKind::Ocean => [0.1, 0.25, 0.55],
    BiomeKind::AlpineTransition => [0.6, 0.5, 0.62],
    BiomeKind::LowerSnowyPeaks => [0.62, 0.78, 0.95],
    BiomeKind::UpperSnowyPeaks => [0.97, 0.99, 1.0],
    BiomeKind::IceArctic => [0.75, 0.92, 1.0],
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::terrain::heightmap::update_stats;
  use crate::terrain::normals::generate_normals;
  use vista_types::TerrainMetadata;

  #[test]
  fn bed_materials_blend_with_the_ground_there() {
    let mut sample = SurfaceSample {
      materials: [0; MATERIAL_COUNT],
      moisture: 0,
      temperature: 0,
      heat: 0,
      occlusion: 0,
      biome: 0,
      forest: 0,
      river: 0,
      permanent_snow: 0,
      celsius_hundredths: 0,
    };
    sample.materials[MAT_LUSH_GRASS] = 255;
    let mut samples = vec![sample; 2];
    apply_bed_materials(&mut samples, &[(1, [102, 0, 0]), (7, [255, 0, 0])]);
    assert_eq!(samples[0].materials[MAT_LUSH_GRASS], 255);
    assert_eq!(samples[1].materials[MAT_LUSH_GRASS], 153);
    assert_eq!(samples[1].materials[MAT_GRAVEL], 102);
  }

  #[test]
  fn low_hills_get_no_automatic_snow_line_below_the_floor() {
    let options = BiomeOptions::default();
    assert_eq!(snow_line_metres(&ramp_map(16, 250.0), &options), 400.0);
    assert_eq!(snow_line_metres(&ramp_map(16, 2000.0), &options), 1600.0);
  }

  fn ramp_map(size: u32, max_height: f32) -> HeightMap {
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 20.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 0.0, metadata);

    for y in 0..size {
      for x in 0..size {
        let h = -20.0 + (x as f32 / (size - 1) as f32) * (max_height + 20.0);
        let _ = map.set_height(x, y, h);
      }
    }

    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    map
  }

  fn classify(map: &HeightMap, options: &BiomeOptions) -> Vec<SurfaceSample> {
    let normals = generate_normals(map);
    classify_surface(map, &normals, None, options)
  }

  #[test]
  fn classification_is_deterministic() {
    let map = ramp_map(64, 1_500.0);
    let options = BiomeOptions::default();

    assert_eq!(classify(&map, &options), classify(&map, &options));
  }

  #[test]
  fn underwater_samples_are_ocean_and_peaks_are_mountains() {
    let map = ramp_map(64, 1_500.0);
    let options = BiomeOptions {
      volcanism: 0.0,
      ..BiomeOptions::default()
    };
    let samples = classify(&map, &options);

    assert_eq!(samples[0].biome_kind(), BiomeKind::Ocean);
    assert_eq!(samples[63].biome_kind(), BiomeKind::UpperSnowyPeaks);
    assert!((0..64).any(|x| samples[x].biome_kind() == BiomeKind::MountainProper));
  }

  #[test]
  fn snowy_peaks_rise_through_transition_lower_and_upper_bands() {
    let map = ramp_map(256, 2_000.0);
    let options = BiomeOptions {
      volcanism: 0.0,
      ..BiomeOptions::default()
    };
    let samples = classify(&map, &options);
    let row = 128 * 256;
    let first = |kind: BiomeKind| (0..256).find(|x| samples[row + x].biome_kind() == kind);
    let transition = first(BiomeKind::AlpineTransition).unwrap();
    let lower = first(BiomeKind::LowerSnowyPeaks).unwrap();
    let upper = first(BiomeKind::UpperSnowyPeaks).unwrap();

    assert!(
      transition < lower && lower < upper,
      "{transition} {lower} {upper}"
    );

    // Snow thickens from patches below the snow line to full cover on top.
    let snow = |x: usize| samples[row + x].materials[MAT_SNOW] as f32 / 255.0;
    assert!(
      snow(transition) < 0.6,
      "transition snow {}",
      snow(transition)
    );
    assert!(snow(255) > 0.85, "summit snow {}", snow(255));
    assert!(snow(upper) >= snow(lower));

    // Nothing grows on the snow, and little in the transition band.
    let forest = |x: usize| samples[row + x].forest;
    assert_eq!(forest(upper), 0);
    assert_eq!(forest(lower), 0);
  }

  #[test]
  fn cold_climates_bring_the_snow_biomes_lower() {
    let map = ramp_map(128, 2_000.0);
    let snowy = |temperature_bias: f32| {
      let options = BiomeOptions {
        volcanism: 0.0,
        temperature_bias,
        ..BiomeOptions::default()
      };
      classify(&map, &options)
        .iter()
        .filter(|sample| {
          matches!(
            sample.biome_kind(),
            BiomeKind::AlpineTransition | BiomeKind::LowerSnowyPeaks | BiomeKind::UpperSnowyPeaks
          )
        })
        .count()
    };

    assert!(snowy(-1.0) > snowy(0.0));
    assert!(snowy(0.0) >= snowy(1.0));
  }

  #[test]
  fn shoreline_is_coastal() {
    let map = ramp_map(256, 3_000.0);
    let options = BiomeOptions {
      volcanism: 0.0,
      beach_height_metres: 8.0,
      ..BiomeOptions::default()
    };
    let samples = classify(&map, &options);
    let coast_x = (0..256)
      .find(|x| map.heights[*x as usize] > 1.0)
      .unwrap_or(0);
    let biome = samples[(128 * 256 + coast_x) as usize].biome_kind();

    assert!(matches!(
      biome,
      BiomeKind::CoastalBeach | BiomeKind::CoastalRocky | BiomeKind::SwampWetlands
    ));
  }

  #[test]
  fn material_weights_sum_to_roughly_one() {
    let map = ramp_map(64, 1_500.0);
    let samples = classify(&map, &BiomeOptions::default());

    for sample in samples {
      let total: u32 = sample.materials.iter().map(|value| *value as u32).sum();
      assert!((250..=262).contains(&total), "total {total}");
    }
  }

  #[test]
  fn hot_and_wet_climates_produce_jungle() {
    let map = ramp_map(128, 4_000.0);
    let options = BiomeOptions {
      temperature_bias: 1.0,
      moisture_bias: 1.0,
      volcanism: 0.0,
      ..BiomeOptions::default()
    };
    let samples = classify(&map, &options);

    assert!(samples.iter().any(|sample| matches!(
      sample.biome_kind(),
      BiomeKind::InnerJungle | BiomeKind::OuterJungle
    )));
  }

  #[test]
  fn hot_and_dry_climates_produce_savannah() {
    let map = ramp_map(128, 4_000.0);
    let options = BiomeOptions {
      temperature_bias: 1.0,
      moisture_bias: -1.0,
      volcanism: 0.0,
      ..BiomeOptions::default()
    };
    let samples = classify(&map, &options);

    assert!(samples
      .iter()
      .any(|sample| sample.biome_kind() == BiomeKind::SavannahExpanse));
  }

  #[test]
  fn volcanism_places_a_caldera_on_a_peak() {
    let size = 128;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 30.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 0.0, metadata);

    for y in 0..size {
      for x in 0..size {
        let dx = x as f32 - 64.0;
        let dy = y as f32 - 64.0;
        let h = (2_000.0 - (dx * dx + dy * dy).sqrt() * 45.0).max(5.0);
        let _ = map.set_height(x, y, h);
      }
    }

    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    let options = BiomeOptions {
      volcanism: 1.0,
      ..BiomeOptions::default()
    };

    assert!(!find_volcanoes(&map, &options).is_empty());
    let samples = classify(&map, &options);
    assert!(samples
      .iter()
      .any(|sample| sample.biome_kind() == BiomeKind::CalderaVolcanic));
  }

  fn flat_land(size: u32, elevation: f32) -> HeightMap {
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 20.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, elevation, metadata);
    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    map
  }

  fn climate(celsius: Option<f32>) -> BiomeOptions {
    BiomeOptions {
      mean_temperature_celsius: celsius,
      volcanism: 0.0,
      ..BiomeOptions::default()
    }
  }

  #[test]
  fn a_frozen_climate_is_an_ice_sheet() {
    let map = flat_land(64, 100.0);
    let samples = classify(&map, &climate(Some(-15.0)));
    let ice = samples
      .iter()
      .filter(|sample| sample.biome_kind() == BiomeKind::IceArctic)
      .count();

    assert!(ice * 10 >= samples.len() * 9, "{ice} of {}", samples.len());
    assert!(samples.iter().all(|sample| sample.celsius() < -8.0));
  }

  #[test]
  fn glacier_samples_hold_permanent_snow_and_ice() {
    let map = flat_land(32, 100.0);
    let samples = classify(&map, &climate(Some(-15.0)));
    let glacier: Vec<_> = samples
      .iter()
      .filter(|sample| sample.is_glacier())
      .collect();

    assert!(!glacier.is_empty());

    for sample in glacier {
      assert_eq!(sample.permanent_snow, 255);
      assert_eq!(sample.forest, 0);
      let frozen = sample.materials[MAT_ICE] as u32 + sample.materials[MAT_SNOW] as u32;
      assert!(frozen > 200, "ice and snow {frozen}");
    }
  }

  /// A broad dome with a gently rounded summit.
  fn dome_map(peak: f32) -> HeightMap {
    let size = 128;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 100.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 0.0, metadata);

    for y in 0..size {
      for x in 0..size {
        let dx = x as f32 - 64.0;
        let dy = y as f32 - 64.0;
        let h = peak * (-(dx * dx + dy * dy) / (30.0 * 30.0)).exp() - 20.0;
        let _ = map.set_height(x, y, h);
      }
    }

    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    map
  }

  #[test]
  fn a_mild_climate_keeps_its_snowy_peaks_and_forms_no_ice() {
    let map = dome_map(3_000.0);
    let samples = classify(&map, &climate(None));

    assert!(samples
      .iter()
      .all(|sample| sample.biome_kind() != BiomeKind::IceArctic && sample.permanent_snow == 0));
    assert_eq!(
      samples[128 * 64 + 64].biome_kind(),
      BiomeKind::UpperSnowyPeaks
    );
    // Summits are just below freezing, the lowlands temperate.
    assert!(samples[128 * 2 + 2].celsius() > 8.0);
    assert!(samples[128 * 64 + 64].celsius() < 0.0);

    // At 15 °C with a real lapse rate, only the upper slopes of a 3000 m
    // peak are cold enough for tundra (below 3 °C) or ice.
    let temperate = classify(&map, &climate(Some(15.0)));

    for (sample, height) in temperate.iter().zip(&map.heights) {
      if sample.biome_kind() == BiomeKind::IceArctic {
        assert!(
          sample.celsius() <= 3.0 && *height > 1_000.0,
          "ice at {height} m, {} °C",
          sample.celsius()
        );
      }
    }

    assert!(temperate
      .iter()
      .any(|sample| sample.biome_kind() == BiomeKind::IceArctic));
  }

  #[test]
  fn a_hot_climate_has_no_ice() {
    let map = ramp_map(128, 3_000.0);
    let samples = classify(&map, &climate(Some(30.0)));

    assert!(samples
      .iter()
      .all(|sample| sample.biome_kind() != BiomeKind::IceArctic));
  }

  #[test]
  fn temperature_falls_with_altitude_at_the_lapse_rate() {
    let map = ramp_map(128, 3_000.0);
    // Climate regions far larger than the map keep the noise constant.
    let options = BiomeOptions {
      climate_scale_metres: 1.0e8,
      ..climate(Some(15.0))
    };
    let samples = classify(&map, &options);
    let row = 64 * 128;
    let low = (samples[row + 20].celsius(), map.heights[row + 20]);
    let high = (samples[row + 120].celsius(), map.heights[row + 120]);
    let rate = (low.0 - high.0) / (high.1 - low.1) * 1_000.0;

    assert!((rate - 6.5).abs() < 0.5, "lapse rate {rate}");
  }

  #[test]
  fn cold_sea_next_to_land_is_fast_ice() {
    let map = ramp_map(64, 400.0);
    let samples = classify(&map, &climate(Some(-18.0)));
    let row = 32 * 64;

    // The ramp crosses sea level about a tenth of the way along.
    assert_eq!(samples[row].biome_kind(), BiomeKind::Ocean);
    assert_eq!(samples[row + 2].permanent_snow, 255);

    let mild = classify(&map, &climate(Some(5.0)));
    assert_eq!(mild[row + 2].permanent_snow, 0);
  }

  #[test]
  fn disabled_biomes_stay_temperate() {
    let map = ramp_map(64, 1_500.0);
    let options = BiomeOptions {
      enabled: false,
      ..BiomeOptions::default()
    };
    let samples = classify(&map, &options);

    assert!(samples.iter().all(|sample| !matches!(
      sample.biome_kind(),
      BiomeKind::InnerJungle
        | BiomeKind::OuterJungle
        | BiomeKind::SavannahExpanse
        | BiomeKind::OuterVolcanic
        | BiomeKind::CalderaVolcanic
        | BiomeKind::IceArctic
    )));
  }
}
