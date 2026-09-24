//! Landform presets: the numbers behind each [`LandformKind`].
//!
//! Every length is in metres. Relief values are the heights a preset
//! reaches on a map large enough to hold its features; smaller maps get
//! proportionally lower relief (see [`Landform::for_extent`]) so slopes
//! stay plausible.

use vista_types::LandformKind;

/// Parameters for one landform preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landform {
  /// Fraction of the map above sea level, 0 to 1.
  pub land_fraction: f32,
  /// Wavelength of the continental shapes.
  pub continent_wavelength: f32,
  /// Wavelength of the mountain ranges.
  pub range_wavelength: f32,
  /// Fraction of the land that carries mountain ranges, 0 to 1.
  pub range_coverage: f32,
  /// Depth of the open sea floor below sea level (negative).
  pub sea_floor: f32,
  /// Height of the rolling lowlands above sea level.
  pub lowland_relief: f32,
  /// Height the ranges add above the lowlands.
  pub mountain_relief: f32,
  /// Stream-power erodibility. 1 is typical; higher carves deeper valleys.
  pub erodibility: f32,
  /// Hillslope diffusion per stream-power iteration, 0 to 0.25.
  pub hillslope_diffusion: f32,
  /// Detail amplitude on plains, as a fraction of the mountain detail.
  pub plains_roughness: f32,
  /// Detail amplitude on mountains, 0 to 2.
  pub mountain_roughness: f32,
  /// Steepest stable slope for loose material, in degrees.
  pub talus_angle_degrees: f32,
  /// Relative rainfall for hydraulic erosion. 1 is typical.
  pub rain: f32,
  /// Strength of plateau terracing, 0 to 1.
  pub terrace: f32,
  /// Whether ranges run straight into the sea as cliffs.
  pub coastal_cliffs: bool,
  /// Strength of glacial carving, 0 to 1.
  pub glacial: f32,
  /// How high a range may stand, as a fraction of its wavelength: the
  /// steepness the landform's rock and climate hold. Glacier-carved alpine
  /// and fjord ranges stand steepest; real 6 km alpine valleys hold 1,500
  /// to 2,200 m of relief.
  pub steepness: f32,
  /// Height of the block the range belt stands on, as a fraction of the
  /// mountain relief. Real ranges rise from high ground: alpine valley
  /// floors lie hundreds of metres up, and the peaks stand above them.
  pub massif: f32,
}

impl Landform {
  /// The preset table.
  pub fn preset(kind: LandformKind) -> Self {
    let base = Self {
      land_fraction: 0.7,
      continent_wavelength: 40_000.0,
      range_wavelength: 14_000.0,
      range_coverage: 0.4,
      sea_floor: -160.0,
      lowland_relief: 220.0,
      mountain_relief: 1400.0,
      erodibility: 1.0,
      hillslope_diffusion: 0.1,
      plains_roughness: 0.08,
      mountain_roughness: 1.0,
      talus_angle_degrees: 38.0,
      rain: 1.0,
      terrace: 0.0,
      coastal_cliffs: false,
      glacial: 0.0,
      massif: 0.0,
      steepness: 0.35,
    };

    match kind {
      LandformKind::Continental => Self {
        massif: 1.0,
        talus_angle_degrees: 47.0,
        ..base
      },
      LandformKind::Alpine => Self {
        land_fraction: 0.95,
        continent_wavelength: 50_000.0,
        range_wavelength: 16_000.0,
        range_coverage: 0.85,
        sea_floor: -200.0,
        lowland_relief: 380.0,
        mountain_relief: 2600.0,
        erodibility: 1.25,
        hillslope_diffusion: 0.08,
        plains_roughness: 0.12,
        mountain_roughness: 1.2,
        talus_angle_degrees: 42.0,
        rain: 1.2,
        glacial: 0.4,
        massif: 0.65,
        steepness: 0.55,
        ..base
      },
      LandformKind::RollingHills => Self {
        land_fraction: 0.9,
        continent_wavelength: 30_000.0,
        range_wavelength: 9_000.0,
        range_coverage: 0.0,
        sea_floor: -80.0,
        lowland_relief: 250.0,
        mountain_relief: 0.0,
        erodibility: 0.8,
        hillslope_diffusion: 0.2,
        plains_roughness: 0.12,
        mountain_roughness: 0.4,
        talus_angle_degrees: 30.0,
        steepness: 0.25,
        ..base
      },
      LandformKind::Archipelago => Self {
        land_fraction: 0.35,
        continent_wavelength: 5_000.0,
        range_wavelength: 8_000.0,
        range_coverage: 0.45,
        sea_floor: -120.0,
        lowland_relief: 140.0,
        mountain_relief: 600.0,
        hillslope_diffusion: 0.12,
        talus_angle_degrees: 36.0,
        rain: 1.2,
        coastal_cliffs: false,
        steepness: 0.4,
        ..base
      },
      LandformKind::MesaDesert => Self {
        land_fraction: 0.97,
        continent_wavelength: 40_000.0,
        range_wavelength: 12_000.0,
        range_coverage: 0.55,
        sea_floor: -100.0,
        lowland_relief: 320.0,
        mountain_relief: 420.0,
        erodibility: 0.7,
        hillslope_diffusion: 0.03,
        plains_roughness: 0.1,
        mountain_roughness: 0.6,
        talus_angle_degrees: 42.0,
        rain: 0.35,
        terrace: 0.7,
        steepness: 0.3,
        ..base
      },
      LandformKind::Fjords => Self {
        land_fraction: 0.75,
        continent_wavelength: 30_000.0,
        range_wavelength: 12_000.0,
        range_coverage: 0.9,
        sea_floor: -260.0,
        lowland_relief: 300.0,
        mountain_relief: 1600.0,
        erodibility: 1.2,
        hillslope_diffusion: 0.06,
        plains_roughness: 0.1,
        mountain_roughness: 1.1,
        talus_angle_degrees: 46.0,
        rain: 1.3,
        coastal_cliffs: true,
        glacial: 1.0,
        massif: 0.8,
        steepness: 0.55,
        ..base
      },
      LandformKind::VolcanicIsland => Self {
        land_fraction: 0.3,
        continent_wavelength: 20_000.0,
        range_wavelength: 6_000.0,
        range_coverage: 0.15,
        sea_floor: -300.0,
        lowland_relief: 120.0,
        mountain_relief: 1500.0,
        erodibility: 1.1,
        hillslope_diffusion: 0.08,
        talus_angle_degrees: 36.0,
        rain: 1.2,
        coastal_cliffs: true,
        steepness: 0.4,
        ..base
      },
    }
  }

  /// The preset fitted to a square map `extent` metres across.
  ///
  /// Continents are at most 1.5 x the extent and ranges at most 0.6 x,
  /// so a small map still holds a coastline and a whole range. Relief is
  /// capped by feature width (ranges at [`Landform::steepness`] of their
  /// wavelength, lowlands at a fortieth of the continent wavelength), so
  /// a shrunken range keeps a plausible slope instead of letting
  /// full-size relief stand on a small footprint.
  pub fn for_extent(kind: LandformKind, extent: f32) -> Self {
    let preset = Self::preset(kind);
    let continent_wavelength = preset.continent_wavelength.min(extent * 1.5);
    let range_wavelength = preset.range_wavelength.min(extent * 0.6);

    Self {
      continent_wavelength,
      range_wavelength,
      mountain_relief: preset
        .mountain_relief
        .min(range_wavelength * preset.steepness),
      lowland_relief: preset.lowland_relief.min(continent_wavelength / 40.0),
      sea_floor: preset.sea_floor.max(-continent_wavelength / 40.0),
      ..preset
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const ALL: [LandformKind; 7] = [
    LandformKind::Continental,
    LandformKind::Alpine,
    LandformKind::RollingHills,
    LandformKind::Archipelago,
    LandformKind::MesaDesert,
    LandformKind::Fjords,
    LandformKind::VolcanicIsland,
  ];

  #[test]
  fn presets_match_their_documented_character() {
    let continental = Landform::preset(LandformKind::Continental);
    assert_eq!(continental.land_fraction, 0.7);
    assert_eq!(continental.mountain_relief, 1400.0);

    let alpine = Landform::preset(LandformKind::Alpine);
    assert_eq!(alpine.land_fraction, 0.95);
    assert_eq!(alpine.mountain_relief, 2600.0);
    assert_eq!(alpine.glacial, 0.4);

    let hills = Landform::preset(LandformKind::RollingHills);
    assert_eq!(hills.range_coverage, 0.0);
    assert_eq!(hills.lowland_relief, 250.0);

    assert_eq!(
      Landform::preset(LandformKind::Archipelago).land_fraction,
      0.35
    );
    assert!(Landform::preset(LandformKind::MesaDesert).terrace >= 0.7);
    assert!(Landform::preset(LandformKind::Fjords).coastal_cliffs);
    assert_eq!(Landform::preset(LandformKind::Fjords).glacial, 1.0);
  }

  #[test]
  fn presets_are_in_range() {
    for kind in ALL {
      let preset = Landform::preset(kind);
      assert!((0.0..=1.0).contains(&preset.land_fraction), "{kind:?}");
      assert!((0.0..=1.0).contains(&preset.range_coverage), "{kind:?}");
      assert!(
        (0.0..=0.25).contains(&preset.hillslope_diffusion),
        "{kind:?}"
      );
      assert!((0.0..=1.0).contains(&preset.glacial), "{kind:?}");
      assert!((0.0..=1.0).contains(&preset.massif), "{kind:?}");
      assert!(preset.sea_floor < 0.0, "{kind:?}");
      assert!(preset.talus_angle_degrees > 0.0 && preset.talus_angle_degrees < 90.0);
    }
  }

  #[test]
  fn wavelengths_and_relief_shrink_with_small_maps() {
    let small = Landform::for_extent(LandformKind::Alpine, 3000.0);
    assert!(small.continent_wavelength <= 4500.5);
    assert!(small.range_wavelength <= 1800.5);
    // Alpine ranges stand at most 0.55 of their wavelength.
    assert!(small.mountain_relief <= 990.5);

    // A 6 km map holds real alpine relief; a 3 km map proportionally less.
    let default = Landform::for_extent(LandformKind::Alpine, 6132.0);
    assert!(default.mountain_relief > 2000.0);
    assert!(small.mountain_relief / default.mountain_relief < 0.5);

    for kind in ALL {
      let fitted = Landform::for_extent(kind, 6132.0);
      assert!(
        fitted.mountain_relief <= fitted.range_wavelength * Landform::preset(kind).steepness + 0.5
      );
    }

    assert!(
      Landform::preset(LandformKind::Continental).steepness
        < Landform::preset(LandformKind::Fjords).steepness
    );

    let large = Landform::for_extent(LandformKind::Alpine, 100_000.0);
    assert_eq!(large, Landform::preset(LandformKind::Alpine));
  }
}
