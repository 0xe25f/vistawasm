//! Tests for the pack ice in `shaders/water.wgsl`, through a CPU port of
//! `pack_ice` and of the noise texture it samples (`gen_noise` in
//! `shaders/texture_gen.wgsl`), at full detail near the camera.

const NOISE_SIZE: usize = 512;

fn pcg(value: u32) -> u32 {
  let state = value.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
  let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
  (word >> 22) ^ word
}

fn unit(h: u32) -> f32 {
  (h & 0xff_ffff) as f32 / 16_777_215.0
}

fn perlin(p: [f32; 2], period: i32, seed: u32) -> f32 {
  let wrap = |v: i32| v.rem_euclid(period) as u32;
  let gradient = |x: i32, y: i32| {
    let h = pcg(wrap(x).wrapping_add(pcg(wrap(y).wrapping_add(pcg(seed)))));
    let angle = unit(h) * std::f32::consts::TAU;
    (angle.cos(), angle.sin())
  };
  let (ix, iy) = (p[0].floor() as i32, p[1].floor() as i32);
  let (fx, fy) = (p[0] - ix as f32, p[1] - iy as f32);
  let fade = |f: f32| f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
  let corner = |dx: i32, dy: i32| {
    let g = gradient(ix + dx, iy + dy);
    g.0 * (fx - dx as f32) + g.1 * (fy - dy as f32)
  };
  let (u, v) = (fade(fx), fade(fy));
  let lerp = crate::maths::lerp;
  lerp(
    lerp(corner(0, 0), corner(1, 0), u),
    lerp(corner(0, 1), corner(1, 1), u),
    v,
  ) * 1.414
}

fn fbm(uv: [f32; 2], period: i32, octaves: i32, seed: u32) -> f32 {
  let (mut value, mut amplitude, mut total, mut frequency) = (0.0, 0.5, 0.0, period);

  for octave in 0..octaves {
    let p = [uv[0] * frequency as f32, uv[1] * frequency as f32];
    value += perlin(p, frequency, seed + octave as u32 * 101) * amplitude;
    total += amplitude;
    amplitude *= 0.5;
    frequency *= 2;
  }

  value / total * 0.5 + 0.5
}

fn remap(value: f32, low: f32, high: f32) -> f32 {
  ((value - low) / (high - low)).clamp(0.0, 1.0)
}

/// The noise texture's r and g channels, at level 0.
struct NoiseTexture(Vec<[f32; 2]>);

impl NoiseTexture {
  fn new() -> Self {
    let texels = (0..NOISE_SIZE * NOISE_SIZE)
      .map(|i| {
        let uv = [
          ((i % NOISE_SIZE) as f32 + 0.5) / NOISE_SIZE as f32,
          ((i / NOISE_SIZE) as f32 + 0.5) / NOISE_SIZE as f32,
        ];
        // Quantised as the rgba8unorm texture stores them.
        let byte = |v: f32| (v * 255.0).round() / 255.0;
        [
          byte(remap(fbm(uv, 4, 6, 301), 0.22, 0.78)),
          byte(remap(fbm(uv, 8, 5, 302), 0.25, 0.75)),
        ]
      })
      .collect();
    Self(texels)
  }

  /// Bilinear, repeating sample of (r, g).
  fn sample(&self, uv: [f32; 2]) -> [f32; 2] {
    let x = uv[0] * NOISE_SIZE as f32 - 0.5;
    let y = uv[1] * NOISE_SIZE as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (x - x0, y - y0);
    let at = |dx: i32, dy: i32| {
      let ix = (x0 as i32 + dx).rem_euclid(NOISE_SIZE as i32) as usize;
      let iy = (y0 as i32 + dy).rem_euclid(NOISE_SIZE as i32) as usize;
      self.0[iy * NOISE_SIZE + ix]
    };
    let (a, b, c, d) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
    std::array::from_fn(|k| {
      let top = a[k] + (b[k] - a[k]) * tx;
      let bottom = c[k] + (d[k] - c[k]) * tx;
      top + (bottom - top) * ty
    })
  }
}

struct Cell {
  f1: f32,
  f2: f32,
  id: f32,
}

/// `floe_hash` in `water.wgsl`.
fn floe_hash(x: f32, y: f32) -> [f32; 3] {
  let fract = |v: f32| v - v.floor();
  let p = [fract(x * 0.1031), fract(y * 0.103), fract(x * 0.0973)];
  let d = p[0] * (p[1] + 33.33) + p[1] * (p[0] + 33.33) + p[2] * (p[2] + 33.33);
  let p = [p[0] + d, p[1] + d, p[2] + d];
  [
    fract((p[0] + p[1]) * p[2]),
    fract((p[0] + p[2]) * p[1]),
    fract((p[1] + p[2]) * p[0]),
  ]
}

/// `floe_cell` in `water.wgsl`.
fn floe_cell(p: [f32; 2]) -> Cell {
  let (bx, by) = (p[0].floor(), p[1].floor());
  let mut cell = Cell {
    f1: 8.0,
    f2: 8.0,
    id: 0.0,
  };

  for y in -1..=1 {
    for x in -1..=1 {
      let (cx, cy) = (bx + x as f32, by + y as f32);
      let random = floe_hash(cx, cy);
      let centre = [cx + random[0] * 0.8 + 0.1, cy + random[1] * 0.8 + 0.1];
      let d = ((p[0] - centre[0]).powi(2) + (p[1] - centre[1]).powi(2)).sqrt();

      if d < cell.f1 {
        cell.f2 = cell.f1;
        cell.f1 = d;
        cell.id = random[2];
      } else if d < cell.f2 {
        cell.f2 = d;
      }
    }
  }

  cell
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
  let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
  t * t * (3.0 - 2.0 * t)
}

fn step(edge: f32, value: f32) -> f32 {
  if value >= edge {
    1.0
  } else {
    0.0
  }
}

/// `floe_layer` in `water.wgsl`, at level 0.
fn floe_layer(noise: &NoiseTexture, p: [f32; 2], size: f32) -> Cell {
  let span = size * 4.0;
  let [r, g] = noise.sample([p[0] / span, p[1] / span]);
  floe_cell([p[0] / size + (r - 0.5) * 0.6, p[1] / size + (g - 0.5) * 0.6])
}

/// `pack_ice` in `water.wgsl` near the camera: (ice cover, lead).
fn pack_ice(noise: &NoiseTexture, p: [f32; 2], c: f32) -> (f32, f32) {
  let large = floe_layer(noise, p, 600.0);
  let gap = 0.03 + (1.0 - c) * 0.03;
  let mut cover = step(large.id, c) * smoothstep(gap, gap + 0.02, large.f2 - large.f1);

  let middle = floe_layer(noise, [p[0] + 37.0, p[1] + 37.0], 120.0);
  let broken = step(middle.id, 0.08 + 0.3 * (1.0 - c));
  let cracked = step(middle.id, 0.35);
  let split = 1.0 + (smoothstep(0.0, 0.02, middle.f2 - middle.f1) - 1.0) * cracked;
  cover *= (1.0 - broken) * split;

  let small = floe_layer(noise, [p[0] + 91.0, p[1] + 91.0], 25.0);
  let cake = step(small.id, c * 0.75) * smoothstep(0.0, 0.02, small.f2 - small.f1);
  cover += (1.0 - cover) * cake;

  let across = -0.6 * p[0] + 0.8 * p[1];
  let along = 0.8 * p[0] + 0.6 * p[1];
  let bend = 150.0 * (along * 0.003 + 0.8 * (across * 0.0009).sin()).sin()
    + 90.0 * (along * 0.0011 + 1.7).sin();
  let q = [along / 16000.0, (across + bend) / 2000.0];
  let [r, g] = noise.sample(q);
  let width = 0.032 + (0.011 - 0.032) * c;
  let lead =
    (1.0 - smoothstep(width * 0.5, width, (r - 0.5).abs())) * smoothstep(0.2, 0.3, g - c * 0.35);
  (cover * (1.0 - lead), lead)
}

/// Sizes of the 4-connected regions where `inside` holds.
fn regions(n: usize, inside: &[bool]) -> Vec<Vec<usize>> {
  let mut seen = vec![false; inside.len()];
  let mut found = Vec::new();

  for start in 0..inside.len() {
    if seen[start] || !inside[start] {
      continue;
    }

    let mut members = Vec::new();
    let mut stack = vec![start];
    seen[start] = true;

    while let Some(i) = stack.pop() {
      members.push(i);
      let (x, y) = (i % n, i / n);

      for (ok, j) in [
        (x > 0, i.wrapping_sub(1)),
        (x + 1 < n, i + 1),
        (y > 0, i.wrapping_sub(n)),
        (y + 1 < n, i + n),
      ] {
        if ok && inside[j] && !seen[j] {
          seen[j] = true;
          stack.push(j);
        }
      }
    }

    found.push(members);
  }

  found
}

#[test]
fn pack_ice_has_floes_of_many_sizes_and_long_leads() {
  let noise = NoiseTexture::new();
  // 4 x 4 km at 4 m, away from the origin.
  let (n, step_metres, origin) = (1000usize, 4.0f32, [12_345.0f32, -6_789.0f32]);
  let mut ice = vec![false; n * n];
  let mut leads = vec![false; n * n];

  for (i, (ice, lead)) in ice.iter_mut().zip(leads.iter_mut()).enumerate() {
    let p = [
      origin[0] + (i % n) as f32 * step_metres,
      origin[1] + (i / n) as f32 * step_metres,
    ];
    let (cover, open) = pack_ice(&noise, p, 0.8);
    *ice = cover > 0.5;
    *lead = open > 0.5;
  }

  let fraction = ice.iter().filter(|i| **i).count() as f32 / ice.len() as f32;
  assert!((fraction - 0.8).abs() <= 0.08, "ice fraction {fraction}");

  let area = step_metres * step_metres;
  let floes: Vec<f32> = regions(n, &ice)
    .iter()
    .filter(|floe| floe.len() >= 3)
    .map(|floe| floe.len() as f32 * area)
    .collect();
  let largest = floes.iter().cloned().fold(0.0, f32::max);
  let smallest = floes.iter().cloned().fold(f32::INFINITY, f32::min);
  assert!(
    largest / smallest >= 100.0,
    "floes from {smallest} to {largest} m2"
  );

  let longest = regions(n, &leads)
    .iter()
    .map(|lead| {
      let xs = lead.iter().map(|i| i % n);
      let ys = lead.iter().map(|i| i / n);
      let width = xs.clone().max().unwrap_or(0) - xs.min().unwrap_or(0);
      let height = ys.clone().max().unwrap_or(0) - ys.min().unwrap_or(0);
      ((width * width + height * height) as f32).sqrt() * step_metres
    })
    .fold(0.0, f32::max);
  assert!(longest > 400.0, "longest lead {longest} m");
  println!(
    "pack ice: {fraction:.3} ice, {} floes from {smallest:.0} to {largest:.0} m2, longest lead {longest:.0} m",
    floes.len()
  );
}
