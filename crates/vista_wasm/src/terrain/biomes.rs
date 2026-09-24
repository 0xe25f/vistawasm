//! Climate-driven biome classification and surface material weights.
//!
//! Every terrain sample is classified into one of the [`BiomeKind`]s from
//! its height, slope, and two seeded, low-frequency climate fields
//! (temperature and moisture), plus a handful of volcanic hotspots placed
//! on the highest peaks. The same pass also produces the eight surface
//! material weights the terrain shader blends between, so ground textures,
//! tree species, and grass colour all agree with each other.
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

/// Number of surface materials.
pub const MATERIAL_COUNT: usize = 8;

/// Compact per-sample surface description, 16 bytes.
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
  /// Reserved.
  pub reserved: u8,
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

        if options.enabled {
          temperature.push(0.56 + t * 0.62 + options.temperature_bias.clamp(-1.0, 1.0) * 0.4);
          moisture.push(0.52 + m * 0.75 + options.moisture_bias.clamp(-1.0, 1.0) * 0.4);
        } else {
          temperature.push(0.46);
          moisture.push(0.5 + m * 0.55);
        }
      }
    }

    Self {
      width,
      height,
      step: step.max(0.0001),
      temperature,
      moisture,
    }
  }

  fn sample(&self, x: u32, y: u32) -> (f32, f32) {
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

    (bilinear(&self.temperature), bilinear(&self.moisture))
  }
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
  let width = map.metadata.width;
  let height = map.metadata.height;
  let count = (width as usize) * (height as usize);

  if count == 0 || normals.len() != count {
    return vec![SurfaceSample::default(); count];
  }

  let sea = map.metadata.sea_level_metres;
  let range = (map.metadata.max_height_metres - sea).max(50.0);
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
  let mut samples = Vec::with_capacity(count);

  for y in 0..height {
    for x in 0..width {
      let index = (y * width + x) as usize;
      let h = map.heights[index];
      let normal = normals[index];
      let steep = (1.0 - normal[1]).clamp(0.0, 1.0);
      let rel = ((h - sea) / range).clamp(0.0, 1.0);
      let above_sea = h - sea;
      let (base_temperature, base_moisture) = climate.sample(x, y);
      let lowland = (1.0 - rel).powi(3);
      let temperature = (base_temperature - rel * 0.62).clamp(0.0, 1.0);
      // A little high-frequency jitter keeps biome borders organic rather
      // than following the smooth climate contours exactly.
      let jitter = hash_noise(detail_seed, x as i32 / 3, y as i32 / 3) * 0.035
        + value_noise(detail_seed, x as f32 * 0.09, y as f32 * 0.09) * 0.05;
      let is_river = river_mask.is_some_and(|mask| mask.get(index).copied().unwrap_or(false));
      let moisture = (base_moisture + lowland * 0.12 + jitter + if is_river { 0.1 } else { 0.0 })
        .clamp(0.0, 1.0);

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
      } else if above_sea < beach * 1.6 && steep > 0.22 {
        BiomeKind::CoastalRocky
      } else if above_sea < beach * 1.6 && !(moisture > 0.72 && temperature > 0.45 && steep < 0.05)
      {
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
      let snow = if snowy {
        let settled = smoothstep((h - snow_line_here) / 60.0 + 0.5);
        (settled.max(patches * 0.6 * into_band) * (1.0 - smoothstep((steep - steep_limit) / 0.25)))
          .max(upper * 0.9)
          .min(1.0)
      } else {
        0.0
      };
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

      if is_river {
        sand = sand.max(0.5);
        mud = mud.max(0.4);
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

      let total: f32 = weights.iter().sum::<f32>().max(0.0001);
      let mut materials = [0u8; MATERIAL_COUNT];

      for (slot, weight) in materials.iter_mut().zip(weights.iter()) {
        *slot = ((weight / total) * 255.0).round().clamp(0.0, 255.0) as u8;
      }

      let forest = forest_density(biome, moisture) * (1.0 - rock.min(1.0)) * (1.0 - snow);

      samples.push(SurfaceSample {
        materials,
        moisture: unit_to_byte(moisture),
        temperature: unit_to_byte(temperature),
        heat: unit_to_byte(caldera_factor * caldera_factor),
        occlusion: unit_to_byte(occlusion),
        biome: biome as u8,
        forest: unit_to_byte(forest),
        river: if is_river { 255 } else { 0 },
        reserved: 0,
      });
    }
  }

  samples
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
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::terrain::heightmap::update_stats;
  use crate::terrain::normals::generate_normals;
  use vista_types::TerrainMetadata;

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
    )));
  }
}
