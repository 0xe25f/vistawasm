// Builds one mip level of an rgba8 texture array from the level above.
//
// Colour is averaged weighted by alpha so transparent texels (the empty
// space around leaves and impostors) do not bleed dark fringes into the
// smaller mips. Alpha-tested layers keep their coverage: `count` first
// tallies, for each layer, how many of the new level's texels would pass
// the 0.5 alpha test at each of 16 alpha scales, and how many of mip 0's
// pass unscaled; `downsample` then writes the level with the scale whose
// share of passing texels comes closest to mip 0's. Distant trees
// therefore keep the density of their foliage instead of thinning into
// skeletons. Opaque textures (terrain, bark) and data (normals, noise)
// are averaged plainly.
//
// `mode`: 0 = sRGB-encoded opaque colour, 1 = sRGB-encoded colour with
// alpha coverage, 2 = linear data (normals, roughness, noise). Layers
// below `opaque_below` are opaque colour, and layers from `linear_from`
// (or odd layers, with `odd_linear`) are linear data.

struct MipParams {
  mode: u32,
  opaque_below: u32,
  linear_from: u32,
  odd_linear: u32,
  // The level written (or counted), 0 for mip 0's own coverage.
  level: u32,
  // Texels in one layer of mip 0.
  reference_texels: u32,
  // Layers to rebuild, one bit each (the first 64), so a bake redoes only
  // what it changed.
  mask_low: u32,
  mask_high: u32,
};

@group(0) @binding(0) var source: texture_2d_array<f32>;
@group(0) @binding(1) var destination: texture_storage_2d_array<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: MipParams;
// Per layer and level, a histogram of the first candidate scale each
// texel passes the alpha test at (16 means none), then mip 0's passing
// texels.
@group(0) @binding(3) var<storage, read_write> counts: array<atomic<u32>>;
// Per species, the canopy's colour (`average_canopy`).
@group(0) @binding(4) var<storage, read_write> canopy: array<vec4<f32>>;

const CANDIDATES: u32 = 16u;
const SLOTS: u32 = 17u;
const LEVELS: u32 = 16u;

var<workgroup> histogram: array<atomic<u32>, 17>;

fn candidate_scale(k: u32) -> f32 {
  return 0.85 + f32(k) * 0.1;
}

fn wanted(layer: u32) -> bool {
  let bits = select(params.mask_high, params.mask_low, layer < 32u);
  return layer >= 64u || ((bits >> (layer % 32u)) & 1u) != 0u;
}

fn layer_mode(layer: u32) -> u32 {
  if (layer < params.opaque_below) {
    return 0u;
  }

  if (layer >= params.linear_from || (params.odd_linear == 1u && layer % 2u == 1u)) {
    return 2u;
  }

  return params.mode;
}

fn slot(layer: u32, level: u32, k: u32) -> u32 {
  return (layer * LEVELS + level) * SLOTS + k;
}

// The plain 2 x 2 average of the level above at `texel`, alpha unscaled.
fn downsampled(texel: vec2<i32>, layer: i32, mode: u32) -> vec4<f32> {
  let base = texel * 2;
  var colour = vec3<f32>(0.0);
  var plain = vec4<f32>(0.0);
  var alpha = 0.0;

  for (var y = 0; y < 2; y = y + 1) {
    for (var x = 0; x < 2; x = x + 1) {
      let value = textureLoad(source, base + vec2<i32>(x, y), layer, 0);
      // Average in linear light so mips keep the right brightness.
      let linear = select(pow(value.rgb, vec3<f32>(2.2)), value.rgb, mode == 2u);
      colour = colour + linear * value.a;
      plain = plain + vec4<f32>(linear, value.a);
      alpha = alpha + value.a;
    }
  }

  if (mode == 1u) {
    let rgb = select(plain.rgb / 4.0, colour / max(alpha, 0.0001), alpha > 0.0001);
    return vec4<f32>(pow(rgb, vec3<f32>(1.0 / 2.2)), alpha / 4.0);
  }

  if (mode == 2u) {
    return plain / 4.0;
  }

  return vec4<f32>(pow(plain.rgb / 4.0, vec3<f32>(1.0 / 2.2)), plain.a / 4.0);
}

// The first candidate scale at which alpha `a` passes the test, or 16.
fn first_passing(a: f32) -> u32 {
  if (a <= 0.0) {
    return CANDIDATES;
  }

  return u32(clamp(ceil((0.5 / a - 0.85) / 0.1 - 0.0001), 0.0, f32(CANDIDATES)));
}

@compute @workgroup_size(8, 8, 1)
fn count(
  @builtin(global_invocation_id) id: vec3<u32>,
  @builtin(workgroup_id) group: vec3<u32>,
  @builtin(local_invocation_index) local: u32
) {
  // The layer is the workgroup's, so every branch on it is uniform.
  let layer = group.z;

  if (layer_mode(layer) != 1u || !wanted(layer)) {
    return;
  }

  if (local < SLOTS) {
    atomicStore(&histogram[local], 0u);
  }

  workgroupBarrier();
  let size = textureDimensions(source);

  // Mip 0's own coverage, the reference; then each level's candidates.
  if (params.level == 0u) {
    if (id.x < size.x && id.y < size.y && textureLoad(source, vec2<i32>(id.xy), i32(layer), 0).a >= 0.5) {
      atomicAdd(&histogram[0], 1u);
    }
  } else if (id.x < size.x / 2u && id.y < size.y / 2u) {
    atomicAdd(&histogram[first_passing(downsampled(vec2<i32>(id.xy), i32(layer), 1u).a)], 1u);
  }

  workgroupBarrier();

  if (local < SLOTS) {
    let count = atomicLoad(&histogram[local]);

    if (count > 0u) {
      let k = select(local, CANDIDATES, params.level == 0u);
      atomicAdd(&counts[slot(layer, params.level, k)], count);
    }
  }
}

@compute @workgroup_size(8, 8, 1)
fn downsample(@builtin(global_invocation_id) id: vec3<u32>) {
  let size = textureDimensions(destination);
  let layer = id.z;

  if (id.x >= size.x || id.y >= size.y || !wanted(layer)) {
    return;
  }

  let mode = layer_mode(layer);
  var result = downsampled(vec2<i32>(id.xy), i32(layer), mode);

  if (mode == 1u) {
    let wanted = f32(atomicLoad(&counts[slot(layer, 0u, CANDIDATES)])) / f32(max(params.reference_texels, 1u));
    let texels = f32(size.x * size.y);
    var best = 1.0;
    var error = 2.0;
    // Texels passing at each scale: the histogram's running total.
    var passing = 0u;

    for (var k = 0u; k < CANDIDATES; k = k + 1u) {
      passing = passing + atomicLoad(&counts[slot(layer, params.level, k)]);
      let share = f32(passing) / texels;

      if (abs(share - wanted) < error) {
        error = abs(share - wanted);
        best = candidate_scale(k);
      }
    }

    result.a = min(result.a * best, 1.0);
  }

  textureStore(destination, vec2<i32>(id.xy), i32(layer), result);
}

// Per species (one invocation each), the canopy layer's colour: the mean
// of its variants' foliage in their views from above, in linear light, at
// the level bound (mip 3, where the normal layer's foliage mark, 1 for
// leaves and 0.5 for bark, averages to above 0.75 where leaves are most).
// From afar a forest shows its leaves: the limbs seen through an open
// crown sit in its shade. `params.mode` holds the variants per
// species.
@compute @workgroup_size(1)
fn average_canopy(@builtin(global_invocation_id) id: vec3<u32>) {
  let species = id.x;
  let size = textureDimensions(source);
  let cell = size / 3u;
  var leaves = vec4<f32>(0.0);
  var shown = vec4<f32>(0.0);

  for (var variant = 0u; variant < params.mode; variant = variant + 1u) {
    let layer = i32((species * params.mode + variant) * 2u);

    // The view from above, the last cell.
    for (var y = cell.y * 2u; y < cell.y * 3u; y = y + 1u) {
      for (var x = cell.x * 2u; x < cell.x * 3u; x = x + 1u) {
        let at = vec2<i32>(i32(x), i32(y));
        let value = textureLoad(source, at, layer, 0);
        let texel = vec4<f32>(pow(value.rgb, vec3<f32>(2.2)), 1.0) * value.a;
        shown = shown + texel;

        if (textureLoad(source, at, layer + 1, 0).a > 0.75) {
          leaves = leaves + texel;
        }
      }
    }
  }

  // A tree without leaves (a replaced model, say) takes all it shows.
  let chosen = select(shown, leaves, leaves.w > 0.0);
  canopy[species] = vec4<f32>(chosen.rgb / max(chosen.w, 0.0001), 0.0);
}
