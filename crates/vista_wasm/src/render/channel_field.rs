//! The channel distance field: the signed distance to the nearest drawn
//! water edge, at four times the heightmap's resolution, so wet margins
//! and river beds follow the drawn banks instead of the heightmap's
//! samples, 12 to 30 m apart.
//!
//! The field is sparse. The map is cut into tiles of [`FIELD_TILE`]
//! texels a side (four heightmap samples), and only tiles within
//! [`FIELD_RANGE_METRES`] of a drawn channel's edge are kept, packed into
//! an atlas the terrain shader reads through a table of slots. Each texel
//! holds two bytes: the signed distance, negative under the water, and
//! the owning channel's speed, whether its banks are rock walls, and the
//! band its loose bed (or rock) covers beside the water. Lakes and plunge pools
//! near a channel are folded in at the heightmap's resolution, so their
//! margins meet the channels' without a seam; elsewhere the wet-bank
//! field still serves them. The terrain shader's twin is
//! `channel_field_at` in `clipmap_render.wgsl`.

use crate::maths::{length2, smoothstep};
use crate::terrain::channels::{rock_banks, ChannelPoint};
use crate::terrain::heightmap::HeightMap;

/// Field texels per heightmap sample along each axis.
pub const FIELD_SCALE: u32 = 4;
/// Texels per tile side.
pub const FIELD_TILE: u32 = 16;
/// The distance the field holds, either side of the water's edge, in
/// metres. Beyond it a texel reads as far from water.
pub const FIELD_RANGE_METRES: f32 = 16.0;
/// Most bytes the atlas and its slot table may take.
pub const FIELD_BUDGET_BYTES: usize = 8 << 20;
/// Atlas tiles per row: the atlas is 1,024 texels wide.
pub const ATLAS_TILES_PER_ROW: u32 = 64;
/// Bytes per texel.
const TEXEL_BYTES: usize = 2;
/// Bytes per tile.
pub const TILE_BYTES: usize = (FIELD_TILE * FIELD_TILE) as usize * TEXEL_BYTES;

/// A tile with nothing in range: every texel far, with no flow.
const EMPTY_TILE: [u8; TILE_BYTES] = {
  let mut tile = [0u8; TILE_BYTES];
  let mut at = 0;

  while at < TILE_BYTES {
    tile[at] = 255;
    at += TEXEL_BYTES;
  }

  tile
};
/// Speed in metres per second per step of a texel's two speed bits.
pub const SPEED_STEP: f32 = 0.5;
/// A loose-bed band of `n` (0 to 15) is `BAND_STEP x n^2` metres wide:
/// fine steps for brooks, up to the field's range.
pub const BAND_STEP: f32 = 0.07;

/// The sparse field (see the module notes).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChannelField {
  /// Map tiles across.
  pub columns: u32,
  /// Map tiles down.
  pub rows: u32,
  /// Per map tile, row by row: its atlas slot plus 1, or 0 where it has
  /// no tile.
  pub slots: Vec<u32>,
  /// Tiles in the atlas, slot by slot, each [`FIELD_TILE`] rows of
  /// [`FIELD_TILE`] texels: the distance byte, then the speed and band
  /// byte (see [`encode_distance`] and [`encode_flow`]).
  pub tiles: Vec<u8>,
  /// Metres per field texel.
  pub texel_metres: f32,
}

/// The distance byte: `d` metres, -[`FIELD_RANGE_METRES`] to
/// [`FIELD_RANGE_METRES`], as 0 to 255.
pub fn encode_distance(d: f32) -> u8 {
  // Never below 0.5 before the cast, which truncates: a rounding.
  ((d / FIELD_RANGE_METRES).clamp(-1.0, 1.0) * 127.5 + 128.0) as u8
}

/// Metres from a distance byte.
pub fn decode_distance(byte: u8) -> f32 {
  (f32::from(byte) - 127.5) * (FIELD_RANGE_METRES / 127.5)
}

/// The flow byte: speed in [`SPEED_STEP`]s in the top two bits (up to
/// 1.5 m/s, past every bed's threshold), then 1 where the stream has bank
/// meshes (`render/water.rs`, narrower than a sample), which draw its
/// margin and face themselves, then 1 where the banks are rock walls
/// ([`rock_banks`] over a half), and in the low four bits the band beside
/// the water the bed (or the rock) covers, as `sqrt(band / BAND_STEP)`
/// rounded up, so a bed never vanishes.
pub fn encode_flow(speed: f32, band: f32, rock: bool, meshed: bool) -> u8 {
  let speed = (speed / SPEED_STEP).round().clamp(0.0, 3.0) as u8;
  let band = (band.max(0.0) / BAND_STEP).sqrt().ceil().min(15.0) as u8;
  speed << 6 | u8::from(meshed) << 5 | u8::from(rock) << 4 | band
}

/// Speed (metres per second), band (metres), rock walls and bank meshes
/// from a flow byte.
pub fn decode_flow(byte: u8) -> (f32, f32, bool, bool) {
  let band = f32::from(byte & 15);
  (
    f32::from(byte >> 6) * SPEED_STEP,
    BAND_STEP * band * band,
    byte & 16 != 0,
    byte & 32 != 0,
  )
}

/// The loose bed beside a channel on one side, in metres from its edge:
/// half its width, and on the inner side of a bend its point bar, out to
/// 1.5 w on tight bends ([`crate::render::water::bed_materials`] stamps
/// the same at the heightmap's resolution). A cut bank's is a quarter of
/// the width.
pub fn loose_band(width: f32, curvature: f32, inner: bool) -> f32 {
  let bar = 1.5 * width * smoothstep(curvature.abs() / 0.2);

  if inner {
    (0.5 * width).max(bar)
  } else {
    0.25 * width + 0.25 * width * (1.0 - smoothstep(curvature.abs() / 0.2))
  }
}

impl ChannelField {
  /// The field around `drawn` channels on `map`, with `lakes` (a
  /// full-resolution mask of lake and pool samples) folded in where a
  /// channel's tile reaches them. Channels are taken widest first, so when
  /// the tiles would overrun [`FIELD_BUDGET_BYTES`] the narrowest
  /// streams are left to the wet-bank field.
  pub fn build(map: &HeightMap, drawn: &[Vec<ChannelPoint>], lakes: &[bool]) -> Self {
    let (width, height) = (map.metadata.width, map.metadata.height);
    let metres = map.metadata.metres_per_sample.max(0.001);

    if width < 2 || height < 2 || drawn.iter().all(|points| points.len() < 2) {
      return Self::default();
    }

    let texel = metres / FIELD_SCALE as f32;
    let span = |samples: u32| ((samples - 1) * FIELD_SCALE).div_ceil(FIELD_TILE) + 1;
    let (columns, rows) = (span(width), span(height));
    let mut slots = vec![0u32; (columns * rows) as usize];
    let table = slots.len() * 4;
    let most = FIELD_BUDGET_BYTES.saturating_sub(table) / TILE_BYTES;
    // The tiles as they will be uploaded: a distance and a flow byte per
    // texel.
    let mut tiles: Vec<u8> = Vec::new();
    let per_tile = (FIELD_TILE * FIELD_TILE) as usize;
    // Widest first, by quarter metres up to 63 m, in the key's top byte
    // (as `build_river_network` orders reaches).
    let mut order: Vec<usize> = drawn
      .iter()
      .enumerate()
      .map(|(index, points)| {
        let widest = points.iter().fold(0.0f32, |w, p| w.max(p.width));
        (255 - (widest * 4.0).min(255.0) as usize) << 24 | index
      })
      .collect();
    order.sort_unstable();
    // In field texels.
    let range = FIELD_RANGE_METRES / texel;
    let limit = [
      ((width - 1) * FIELD_SCALE) as i32,
      ((height - 1) * FIELD_SCALE) as i32,
    ];

    let scale = FIELD_SCALE as f32;

    for points in order.iter().map(|key| &drawn[key & 0xff_ffff]) {
      // Each point's flow bytes, right and left of the direction of flow.
      let flows: Vec<[u8; 2]> = points.iter().map(|p| point_flows(p, metres)).collect();
      let mut i = 0;

      while i + 1 < points.len() {
        let k = run_end(points, &flows, i, 0.25 / scale, metres);
        // The run before ended with a disc round this point.
        let first = i == 0;
        let (a, b) = (&points[i], &points[k]);
        let side_flows = flows[i];
        i = k;

        if a.falling && b.falling {
          continue;
        }

        let (ax, ay, bx, by) = (a.x * scale, a.y * scale, b.x * scale, b.y * scale);
        let half = [0.5 * a.width / texel, 0.5 * b.width / texel];
        // Out to where anything changes, past the wet margin, the bed's
        // band and the bankside plants, then a texel more to read between:
        // beyond, a texel reads as far.
        let band = decode_flow(side_flows[0])
          .1
          .max(decode_flow(side_flows[1]).1);
        let wanted = band.max(2.0).max(4.0 + 0.5 * a.width.max(b.width)) / texel + 1.0;
        let range = wanted.min(range);
        let reach = half[0].max(half[1]) + range;
        let segment = [bx - ax, by - ay];
        let inverse = 1.0 / (segment[0] * segment[0] + segment[1] * segment[1]).max(1e-12);
        let y0 = ((ay.min(by) - reach).floor() as i32).max(0);
        let y1 = ((ay.max(by) + reach).ceil() as i32).min(limit[1]);

        for y in y0..=y1 {
          let py = y as f32 - ay;
          let Some((low, high)) = capsule_row([ax, ay], [bx, by], reach, y as f32, first) else {
            continue;
          };
          let row = (y as u32 / FIELD_TILE * columns) as usize;
          let local_row = (y as u32 % FIELD_TILE * FIELD_TILE) as usize;
          let (x0, x1) = (
            (low.floor() as i32).max(0),
            (high.ceil() as i32).min(limit[0]),
          );
          // The projection along the segment grows by a fixed step per texel.
          let start = ((x0 as f32 - ax) * segment[0] + py * segment[1]) * inverse;
          let step = segment[0] * inverse;

          for x in x0..=x1 {
            let px = x as f32 - ax;
            let t = (start + step * (x - x0) as f32).clamp(0.0, 1.0);
            let (dx, dy) = (px - segment[0] * t, py - segment[1] * t);
            let squared = dx * dx + dy * dy;
            let half = half[0] + (half[1] - half[0]) * t;

            // Out of range: no square root needed to know.
            if squared >= (range + half) * (range + half) {
              continue;
            }

            let tile = row + (x as u32 / FIELD_TILE) as usize;

            if slots[tile] == 0 {
              if tiles.len() / TILE_BYTES >= most {
                continue;
              }

              tiles.extend_from_slice(&EMPTY_TILE);
              slots[tile] = (tiles.len() / TILE_BYTES) as u32;
            }

            let at = ((slots[tile] as usize - 1) * per_tile
              + local_row
              + (x as u32 % FIELD_TILE) as usize)
              * TEXEL_BYTES;
            // Only a nearer edge than the one found needs the square root.
            let best = decode_distance(tiles[at]) / texel + half;

            if best <= 0.0 || squared >= best * best {
              continue;
            }

            tiles[at] = encode_distance((squared.sqrt() - half) * texel);
            tiles[at + 1] = side_flows[usize::from(segment[0] * py - segment[1] * px > 0.0)];
          }
        }
      }
    }

    if tiles.is_empty() {
      return Self::default();
    }

    fold_lakes(map, lakes, &slots, columns, &mut tiles);
    Self {
      columns,
      rows,
      slots,
      tiles,
      texel_metres: texel,
    }
  }

  /// The atlas as an image [`ATLAS_TILES_PER_ROW`] tiles wide, row by
  /// row, two bytes a texel, and its height in texels: what the terrain
  /// shader samples. At least one tile row, empty without tiles.
  pub fn atlas_image(&self) -> (Vec<u8>, u32) {
    let tile = FIELD_TILE as usize;
    let per_row = ATLAS_TILES_PER_ROW as usize;
    let rows = self.tile_count().div_ceil(per_row).max(1);
    let stride = per_row * tile * TEXEL_BYTES;
    let mut image = vec![0u8; rows * tile * stride];

    for (slot, texels) in self.tiles.chunks_exact(TILE_BYTES).enumerate() {
      let (column, row) = (slot % per_row, slot / per_row);

      for (line, bytes) in texels.chunks_exact(tile * TEXEL_BYTES).enumerate() {
        let at = (row * tile + line) * stride + column * tile * TEXEL_BYTES;
        image[at..at + bytes.len()].copy_from_slice(bytes);
      }
    }

    (image, (rows * tile) as u32)
  }

  /// The words the terrain shader reads the slots from: field texels per
  /// metre (as f32 bits), map tiles across and down, and 1 when there
  /// are tiles, then [`ChannelField::slots`].
  pub fn slot_words(&self) -> Vec<u32> {
    let mut words = vec![
      (1.0 / self.texel_metres.max(1e-6)).to_bits(),
      self.columns,
      self.rows,
      u32::from(!self.tiles.is_empty()),
    ];
    words.extend_from_slice(&self.slots);
    words
  }

  /// Tiles in the atlas.
  pub fn tile_count(&self) -> usize {
    self.tiles.len() / TILE_BYTES
  }

  /// Bytes the atlas and its slot table take on the GPU.
  pub fn bytes(&self) -> usize {
    self.tiles.len() + self.slots.len() * 4
  }

  /// The texel at field coordinates `(x, y)`, or `None` without a tile.
  pub fn texel(&self, x: i32, y: i32) -> Option<[u8; 2]> {
    let (tx, ty) = (
      x.div_euclid(FIELD_TILE as i32),
      y.div_euclid(FIELD_TILE as i32),
    );

    if x < 0 || y < 0 || tx >= self.columns as i32 || ty >= self.rows as i32 {
      return None;
    }

    let slot = *self
      .slots
      .get((ty as u32 * self.columns + tx as u32) as usize)?;
    let local = (y as u32 % FIELD_TILE * FIELD_TILE + x as u32 % FIELD_TILE) as usize;
    let at = ((slot.checked_sub(1)? as usize) * (FIELD_TILE * FIELD_TILE) as usize + local) * 2;
    Some([self.tiles[at], self.tiles[at + 1]])
  }

  /// The distance in metres to the water's edge at heightmap sample
  /// coordinates `(x, y)`, read between texels as the shader reads it,
  /// and the flow byte of the nearest texel; `None` off the tiles.
  pub fn at(&self, x: f32, y: f32) -> Option<(f32, u8)> {
    let (fx, fy) = (x * FIELD_SCALE as f32, y * FIELD_SCALE as f32);
    let (bx, by) = (fx.floor(), fy.floor());
    let (u, v) = (fx - bx, fy - by);
    let read = |dx: i32, dy: i32| {
      self
        .texel(bx as i32 + dx, by as i32 + dy)
        .map_or(FIELD_RANGE_METRES, |texel| decode_distance(texel[0]))
    };
    let top = read(0, 0) + (read(1, 0) - read(0, 0)) * u;
    let bottom = read(0, 1) + (read(1, 1) - read(0, 1)) * u;
    let flow = self.texel(fx.round() as i32, fy.round() as i32)?;
    Some((top + (bottom - top) * v, flow[1]))
  }
}

/// A point's flow bytes for its right and left banks (looking
/// downstream), on samples `metres` apart: rock walls up a width of the
/// bank, or the loose bed, wider on the inner side of a bend.
fn point_flows(p: &ChannelPoint, metres: f32) -> [u8; 2] {
  let rock = rock_banks(p) > 0.5;
  let side = |left: bool| {
    let band = if rock {
      p.width.max(2.0)
    } else {
      loose_band(p.width, p.curvature, left == (p.curvature > 0.0))
    };
    encode_flow(p.speed, band, rock, p.width < metres && !p.falling)
  };
  [side(false), side(true)]
}

/// The last point of the run from point `i` the field draws as one
/// segment: at most 32 points on, while every point between lies within
/// `tolerance` samples of the chord, its width as close to the chord's
/// (on samples `metres` apart), and its flow bytes and fall the same. A
/// segment's cost is its range either side, whatever its length, so long
/// runs are cheap.
fn run_end(
  points: &[ChannelPoint],
  flows: &[[u8; 2]],
  i: usize,
  tolerance: f32,
  metres: f32,
) -> usize {
  let a = &points[i];
  let mut end = i + 1;

  for k in i + 2..points.len().min(i + 33) {
    let c = &points[k];
    let chord = [c.x - a.x, c.y - a.y];
    let length = length2(chord[0], chord[1]).max(1e-6);
    let fits = (i + 1..k).all(|j| {
      let p = &points[j];
      let (px, py) = (p.x - a.x, p.y - a.y);
      let along = ((px * chord[0] + py * chord[1]) / (length * length)).clamp(0.0, 1.0);
      let width = a.width + (c.width - a.width) * along;
      (px * chord[1] - py * chord[0]).abs() / length <= tolerance
        && (p.width - width).abs() <= tolerance * metres
        && flows[j] == flows[i]
        && p.falling == a.falling
    }) && flows[k] == flows[i]
      && c.falling == a.falling;

    if !fits {
      break;
    }

    end = k;
  }

  end
}

/// The span of field row `y` within `reach` of the segment from `a` to
/// `b`, all in field texels, or `None` where the row misses it: the
/// union of its end discs (`a`'s only with `start`) and the band between
/// them, which, the shape being convex, is one span.
fn capsule_row(a: [f32; 2], b: [f32; 2], reach: f32, y: f32, start: bool) -> Option<(f32, f32)> {
  let mut span = (f32::MAX, f32::MIN);
  let mut add = |low: f32, high: f32| {
    if low <= high {
      span = (span.0.min(low), span.1.max(high));
    }
  };

  for end in [a, b].into_iter().skip(usize::from(!start)) {
    let dy = y - end[1];

    if dy.abs() <= reach {
      let w = (reach * reach - dy * dy).sqrt();
      add(end[0] - w, end[0] + w);
    }
  }

  let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
  let length = length2(dx, dy);

  if length > 1e-6 {
    let (ux, uy) = (dx / length, dy / length);
    let rise = y - a[1];
    // Within `reach` across the segment's line: `|(x - ax) uy - rise ux| <= reach`.
    let across = if uy.abs() > 1e-6 {
      let (p, q) = (
        a[0] + (rise * ux - reach) / uy,
        a[0] + (rise * ux + reach) / uy,
      );
      (p.min(q), p.max(q))
    } else if (rise * ux).abs() <= reach {
      (f32::MIN, f32::MAX)
    } else {
      (f32::MAX, f32::MIN)
    };
    // Between its ends: `0 <= (x - ax) ux + rise uy <= length`.
    let along = if ux.abs() > 1e-6 {
      let (p, q) = (a[0] - rise * uy / ux, a[0] + (length - rise * uy) / ux);
      (p.min(q), p.max(q))
    } else if (0.0..=length).contains(&(rise * uy)) {
      (f32::MIN, f32::MAX)
    } else {
      (f32::MAX, f32::MIN)
    };
    add(across.0.max(along.0), across.1.min(along.1));
  }

  (span.0 <= span.1).then_some(span)
}

/// Fold the distance to lake and pool samples (their edge half a sample
/// out) into the tiles there are: a chamfer at the heightmap's
/// resolution, read between samples.
fn fold_lakes(map: &HeightMap, lakes: &[bool], slots: &[u32], columns: u32, tiles: &mut [u8]) {
  if !lakes.contains(&true) {
    return;
  }

  let (width, height) = (map.metadata.width as usize, map.metadata.height as usize);
  let metres = map.metadata.metres_per_sample.max(0.001);
  // Only the lakes' window, grown by the field's range: no tile further
  // off can come within range of them.
  let margin = (FIELD_RANGE_METRES / metres).ceil() as usize + 1;
  let (mut low, mut high) = ([usize::MAX; 2], [0usize; 2]);

  for (index, _) in lakes.iter().enumerate().filter(|(_, wet)| **wet) {
    let (x, y) = (index % width, index / width);
    low = [low[0].min(x), low[1].min(y)];
    high = [high[0].max(x), high[1].max(y)];
  }

  let low = [low[0].saturating_sub(margin), low[1].saturating_sub(margin)];
  let high = [
    (high[0] + margin).min(width - 1),
    (high[1] + margin).min(height - 1),
  ];
  let window = high[0] - low[0] + 1;
  let mut lake = Vec::with_capacity(window * (high[1] - low[1] + 1));

  for y in low[1]..=high[1] {
    for x in low[0]..=high[0] {
      lake.push(if lakes[y * width + x] {
        -0.5 * metres
      } else {
        f32::MAX
      });
    }
  }

  crate::terrain::biomes::chamfer_distance(window, high[1] - low[1] + 1, metres, &mut lake);
  // Off the window, as far as the field reaches.
  let at = |x: usize, y: usize| {
    if x < low[0] || y < low[1] || x > high[0] || y > high[1] {
      FIELD_RANGE_METRES
    } else {
      lake[(y - low[1]) * window + x - low[0]].min(FIELD_RANGE_METRES)
    }
  };
  let per_tile = (FIELD_TILE * FIELD_TILE) as usize;
  let scale = FIELD_SCALE as f32;
  let samples = (FIELD_TILE / FIELD_SCALE) as usize;

  for (tile, slot) in slots.iter().enumerate().filter(|(_, slot)| **slot > 0) {
    let (tx, ty) = (tile as u32 % columns, tile as u32 / columns);
    let (sx, sy) = (tx as usize * samples, ty as usize * samples);
    // Most tiles are far from any lake: the samples at and around them say
    // so.
    let mut near = false;

    for y in sy..=(sy + samples).min(height - 1) {
      for x in sx..=(sx + samples).min(width - 1) {
        near |= at(x, y) < FIELD_RANGE_METRES;
      }
    }

    if !near {
      continue;
    }

    for local in 0..per_tile as u32 {
      let x = (tx * FIELD_TILE + local % FIELD_TILE) as f32 / scale;
      let y = (ty * FIELD_TILE + local / FIELD_TILE) as f32 / scale;
      let (sx, sy) = (
        (x.floor() as usize).min(width - 1),
        (y.floor() as usize).min(height - 1),
      );
      let (nx, ny) = ((sx + 1).min(width - 1), (sy + 1).min(height - 1));
      let (u, v) = (x - sx as f32, y - sy as f32);
      let top = at(sx, sy) + (at(nx, sy) - at(sx, sy)) * u;
      let bottom = at(sx, ny) + (at(nx, ny) - at(sx, ny)) * u;
      let d = top + (bottom - top) * v;
      let at = (*slot as usize - 1) * per_tile + local as usize;

      if d < decode_distance(tiles[at * TEXEL_BYTES]) {
        // Still water: no loose bed, and slow.
        tiles[at * TEXEL_BYTES..at * TEXEL_BYTES + 2].copy_from_slice(&[encode_distance(d), 0]);
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vista_types::TerrainMetadata;

  fn flat(size: u32, metres: f32) -> HeightMap {
    HeightMap::flat(
      size,
      size,
      10.0,
      TerrainMetadata {
        width: size,
        height: size,
        metres_per_sample: metres,
        ..TerrainMetadata::default()
      },
    )
  }

  fn point(x: f32, y: f32, width: f32, speed: f32) -> ChannelPoint {
    ChannelPoint {
      x,
      y,
      width,
      speed,
      level: 10.0,
      depth: 0.5,
      ..ChannelPoint::default()
    }
  }

  #[test]
  fn bytes_round_trip() {
    for d in [-16.0, -3.2, 0.0, 0.7, 15.9] {
      assert!(
        (decode_distance(encode_distance(d)) - d).abs() < 0.07,
        "{d}"
      );
    }

    let (speed, band, rock, meshed) = decode_flow(encode_flow(1.3, 2.0, false, true));
    assert!(
      speed == 1.5 && (2.0..2.6).contains(&band) && !rock && meshed,
      "{band}"
    );
    let (speed, band, rock, meshed) = decode_flow(encode_flow(9.0, 99.0, true, false));
    assert!(speed == 1.5 && (band - BAND_STEP * 225.0).abs() < 1e-4 && rock && !meshed);
  }

  /// A brook 1.6 m wide, far narrower than a 12 m sample, wandering
  /// across the map.
  fn brook() -> (HeightMap, Vec<ChannelPoint>) {
    let points = (0..=80)
      .map(|i| {
        let x = 4.0 + i as f32 * 0.5;
        point(x, 20.0 + 3.0 * (x / 7.0).sin(), 1.6, 0.8)
      })
      .collect();
    (flat(64, 12.0), points)
  }

  #[test]
  fn the_field_reads_zero_at_the_drawn_edge() {
    let (map, points) = brook();
    let field = ChannelField::build(&map, std::slice::from_ref(&points), &[]);
    let metres = map.metadata.metres_per_sample;

    for pair in points.windows(2).step_by(3) {
      let (a, b) = (&pair[0], &pair[1]);
      let (dx, dy) = (b.x - a.x, b.y - a.y);
      let length = length2(dx, dy);
      let normal = [-dy / length, dx / length];

      for side in [-1.0f32, 1.0] {
        let out = 0.5 * a.width / metres * side;
        let (x, y) = (a.x + normal[0] * out, a.y + normal[1] * out);
        let (d, _) = field.at(x, y).expect("a tile at the brook");
        // Within a tenth of a sample.
        assert!(d.abs() < 0.1 * metres, "{d} m at {x}, {y}");
      }

      // Outside the water it is positive.
      let far = 4.0 / metres;
      assert!(
        field
          .at(a.x + normal[0] * far, a.y + normal[1] * far)
          .unwrap()
          .0
          > 2.0
      );
    }
  }

  #[test]
  fn the_field_is_negative_under_water_wider_than_its_texels() {
    let (map, mut points) = brook();
    points.iter_mut().for_each(|p| p.width = 10.0);
    let field = ChannelField::build(&map, std::slice::from_ref(&points), &[]);

    for p in &points[2..points.len() - 2] {
      assert!(field.at(p.x, p.y).unwrap().0 < -3.0);
    }
  }

  #[test]
  fn the_field_is_empty_away_from_water() {
    let (map, points) = brook();
    let field = ChannelField::build(&map, &[points], &[]);
    // Rows 40 and on are over 200 m from the brook.
    assert!(field.at(30.0, 50.0).is_none());
    assert!(field.slots.iter().filter(|slot| **slot > 0).count() < field.slots.len() / 3);
    assert!(ChannelField::build(&map, &[], &[]).tiles.is_empty());
  }

  #[test]
  fn texels_carry_their_channels_speed_and_a_wider_bed_inside_bends() {
    let (map, mut points) = brook();
    let field = ChannelField::build(&map, std::slice::from_ref(&points), &[]);
    let (_, flow) = field.at(points[10].x, points[10].y).unwrap();
    let (speed, _, _, meshed) = decode_flow(flow);
    assert!(speed == 1.0 && meshed);
    assert!(loose_band(10.0, 0.5, true) > 2.0 * loose_band(10.0, 0.5, false));

    // A slow stream reads slow.
    points.iter_mut().for_each(|p| p.speed = 0.2);
    let field = ChannelField::build(&map, &[points.clone()], &[]);
    assert_eq!(
      decode_flow(field.at(points[10].x, points[10].y).unwrap().1).0,
      0.0
    );
  }

  #[test]
  fn powerful_steep_reaches_mark_rock_walls() {
    let (map, mut points) = brook();
    let field = ChannelField::build(&map, std::slice::from_ref(&points), &[]);
    assert!(!decode_flow(field.at(points[10].x, points[10].y).unwrap().1).2);

    for p in &mut points {
      (p.width, p.slope, p.discharge, p.speed) = (6.0, 0.06, 20.0, 3.0);
    }

    let field = ChannelField::build(&map, std::slice::from_ref(&points), &[]);
    let (_, band, rock, _) = decode_flow(field.at(points[10].x, points[10].y).unwrap().1);
    assert!(rock && band >= 6.0, "{band}");
  }

  #[test]
  fn lakes_by_a_channel_join_its_field() {
    let (map, points) = brook();
    let mut lakes = vec![false; map.heights.len()];
    // A lake at the brook's end.
    for y in 18..24 {
      for x in 44..50 {
        lakes[y * 64 + x] = true;
      }
    }
    let field = ChannelField::build(&map, &[points], &lakes);
    let (d, flow) = field.at(46.0, 21.0).unwrap();
    assert!(d < 0.0 && flow == 0, "{d}");
  }

  #[test]
  fn the_atlas_image_holds_each_tile_in_its_slot() {
    let (map, points) = brook();
    let field = ChannelField::build(&map, &[points], &[]);
    let (image, height) = field.atlas_image();
    let stride = (ATLAS_TILES_PER_ROW * FIELD_TILE) as usize * 2;
    assert_eq!(image.len(), stride * height as usize);
    let words = field.slot_words();
    assert_eq!(f32::from_bits(words[0]), 4.0 / 12.0);
    assert_eq!(&words[4..], &field.slots[..]);

    // Every texel the field reads is where the shader looks for it.
    for (tile, slot) in field.slots.iter().enumerate().filter(|(_, s)| **s > 0) {
      let (tx, ty) = (tile as u32 % field.columns, tile as u32 / field.columns);
      let slot = *slot - 1;
      let (ax, ay) = (
        slot % ATLAS_TILES_PER_ROW * FIELD_TILE,
        slot / ATLAS_TILES_PER_ROW * FIELD_TILE,
      );

      for (lx, ly) in [(0, 0), (15, 3), (7, 15)] {
        let texel = field
          .texel((tx * FIELD_TILE + lx) as i32, (ty * FIELD_TILE + ly) as i32)
          .unwrap();
        let at = (ay + ly) as usize * stride + (ax + lx) as usize * 2;
        assert_eq!(texel, [image[at], image[at + 1]]);
      }
    }
  }

  #[test]
  fn a_1024_map_full_of_streams_fits_the_budget() {
    // Streams every 6 samples across a 1024 x 1024 map at 30 m: far more
    // than any real map carries.
    let map = flat(1024, 30.0);
    let streams: Vec<Vec<ChannelPoint>> = (0..170)
      .map(|row| {
        (0..=1000)
          .map(|i| {
            point(
              10.0 + i as f32,
              6.0 * row as f32 + 2.0,
              3.0 + row as f32 * 0.01,
              1.0,
            )
          })
          .collect()
      })
      .collect();
    let field = ChannelField::build(&map, &streams, &[]);
    assert!(field.bytes() <= FIELD_BUDGET_BYTES, "{}", field.bytes());
    // The widest streams were kept first.
    let widest = streams.last().unwrap();
    assert!(field.at(widest[500].x, widest[500].y).is_some());
  }
}
