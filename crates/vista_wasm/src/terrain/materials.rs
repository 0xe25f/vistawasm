use crate::maths::clamp_f32;
use crate::terrain::heightmap::HeightMap;
use crate::terrain::normals::generate_normals;

/// Material weights for one terrain sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialWeights {
  /// Grass weight.
  pub grass: f32,
  /// Rock weight.
  pub rock: f32,
  /// Snow weight.
  pub snow: f32,
  /// Wet mud weight.
  pub wet_mud: f32,
}

/// Generate simple material masks from height and slope.
pub fn generate_material_masks(map: &HeightMap, snow_line_metres: f32) -> Vec<MaterialWeights> {
  let normals = generate_normals(map);

  map
    .heights
    .iter()
    .zip(normals.iter())
    .map(|(height, normal)| {
      let slope = (1.0 - normal[1]).clamp(0.0, 1.0);
      let snow = clamp_f32((*height - snow_line_metres) / 350.0, 0.0, 1.0);
      let rock = clamp_f32(slope * 1.8, 0.0, 1.0) * (1.0 - snow * 0.6);
      let wet_mud = clamp_f32(
        (map.metadata.sea_level_metres + 8.0 - *height) / 24.0,
        0.0,
        1.0,
      );
      let grass = (1.0 - rock - snow * 0.75 - wet_mud * 0.4).clamp(0.0, 1.0);

      MaterialWeights {
        grass,
        rock,
        snow,
        wet_mud,
      }
    })
    .collect()
}
