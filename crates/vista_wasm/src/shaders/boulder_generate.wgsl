// Streamed boulders: `generate_main` fills 32 m tiles within the boulder
// distance, one invocation per lattice point (2 m apart), and `cull_main`
// picks the boulders to draw this frame. `ground.wgsl`, `lattice.wgsl`
// and `generate_common.wgsl` come first.
//
// Generation mirrors `boulder_at` in `render/boulders.rs`: a boulder
// where the point's rank is below the talus field's probability, sized by
// a power law, never in water, in a drawn channel or off the map, grounded
// on the drawn mesh and sunk by 25 to 40 % of its height, leaning with the
// slope. Each invocation then places the stream stones (`stone_at`) of
// the four 1 m cells within its 2 m cell, sunk by 30 to 60 % into the
// bed.
//
// Culling keeps the boulders in the view, at most `MAX_BOULDERS`. From
// 0.8 of the boulder distance out they drop out by rank, shrinking to
// nothing over a band of ranks, so none pops. Each goes in the list of
// its variant and level of detail (by how large it looks), and within the
// shadow distance in its variant's shadow list too.

struct BoulderParams {
  terrain: vec4<f32>,
  terrain2: vec4<f32>,
  mesh: vec4<f32>,
  // x: 1 / metres per texel, y: the boulder distance, z: the shadow
  // distance, w: pixels per radian, below which a boulder is too small to
  // draw.
  rules: vec4<f32>,
  // x: lattice seed, y: pool entries, z: entries per slot, w: the stream
  // stones' lattice seed.
  shape: vec4<u32>,
  // xyz: camera.
  camera: vec4<f32>,
  // Each variant's height per metre across (`BoulderMeshes::heights`).
  tall: array<vec4<f32>, 2>,
  planes: array<vec4<f32>, 6>,
};

@group(0) @binding(0) var<uniform> params: BoulderParams;
// Boulders as raw floats, 8 each (`Boulder` in `render/boulders.rs`).
@group(0) @binding(7) var<storage, read_write> pool: array<f32>;
@group(0) @binding(8) var<storage, read_write> counts: array<atomic<u32>>;
@group(0) @binding(10) var<storage, read_write> drawn: array<f32>;
@group(0) @binding(11) var<storage, read_write> args: array<atomic<u32>>;

const VARIANTS: u32 = 6u;
const MAX_BOULDERS: u32 = 20000u;
// `boulders::LIST_CAPACITY` and `SHADOW_CAPACITY`: each variant's lists,
// finest level first, then the shadow lists after every variant's.
const VARIANT_ENTRIES: u32 = 7168u;
const SHADOW_CAPACITY: u32 = 2048u;
// The word counting every boulder drawn, after the 24 indexed draws.
const TOTAL: u32 = 120u;

@compute @workgroup_size(8, 8)
fn generate_main(@builtin(global_invocation_id) id: vec3<u32>) {
  let job = jobs[id.z];
  let first = vec2<i32>(tile_first(job.tile.x, 32, 2), tile_first(job.tile.y, 32, 2));
  let next = vec2<i32>(tile_first(job.tile.x + 1, 32, 2), tile_first(job.tile.y + 1, 32, 2));
  let cell = first + vec2<i32>(id.xy);

  if (any(cell >= next)) {
    return;
  }

  let m = Mapping(params.terrain, params.terrain2, params.rules.x);
  let b = boulder_candidate(m, cell, params.shape.x);

  if (b.z > 0.0) {
    let traits = pcg3d(vec3<u32>(bitcast<vec2<u32>>(cell), params.shape.x ^ 0x2545f491u));
    let variant = traits.z % VARIANTS;
    let sink = (0.25 + 0.15 * unit(traits.y)) * b.z * params.tall[variant / 4u][variant % 4u];
    let span = vec2<f32>(SLOPE_SPAN, 0.0);
    let lean = vec2<f32>(
      height_at(m, b.xy + span.xy) - height_at(m, b.xy - span.xy),
      height_at(m, b.xy + span.yx) - height_at(m, b.xy - span.yx)
    ) / (2.0 * SLOPE_SPAN);
    store(job, b, sink, lean, variant);
  }

  for (var k = 0; k < 4; k = k + 1) {
    let stone_cell = 2 * cell + vec2<i32>(k % 2, k / 2);
    let s = stone_candidate(stone_cell, params.shape.w);

    if (s.z > 0.0) {
      let traits = pcg3d(vec3<u32>(bitcast<vec2<u32>>(stone_cell), params.shape.w ^ 0x2545f491u));
      let variant = traits.z % VARIANTS;
      let sink = (0.3 + 0.3 * unit(traits.y)) * s.z * params.tall[variant / 4u][variant % 4u];
      store(job, s, sink, vec2<f32>(0.0), variant);
    }
  }
}

// Store boulder `b` (x, z, size and relative rank), sunk by `sink` and
// leaning by `lean`, in `job`'s slot.
fn store(job: Job, b: vec4<f32>, sink: f32, lean: vec2<f32>, variant: u32) {
  let slot = atomicAdd(&counts[job.slot], 1u);

  if (slot < job.capacity) {
    let base = (job.first + slot) * 8u;
    pool[base] = b.x;
    pool[base + 1u] = grounded_base(heights, params.terrain, params.terrain2, params.mesh, b.xy, 0.0) - sink;
    pool[base + 2u] = b.y;
    pool[base + 3u] = b.z;
    pool[base + 4u] = sink;
    pool[base + 5u] = lean.x;
    pool[base + 6u] = lean.y;
    pool[base + 7u] = f32(variant) + min(b.w, 0.999);
  }
}

// Append the boulder at float `source` of the pool to draw list `list`,
// which starts at entry `start` and holds `capacity`, at `scale`.
fn append(list: u32, start: u32, capacity: u32, source: u32, scale: f32) {
  let place = atomicAdd(&args[list * 5u + 1u], 1u);

  if (place >= capacity) {
    atomicSub(&args[list * 5u + 1u], 1u);
    return;
  }

  let out = (start + place) * 8u;

  for (var k = 0u; k < 8u; k = k + 1u) {
    drawn[out + k] = pool[source + k];
  }

  drawn[out + 3u] = scale;
}

@compute @workgroup_size(64)
fn cull_main(@builtin(global_invocation_id) id: vec3<u32>) {
  let index = id.x;
  let capacity = params.shape.z;

  if (index >= params.shape.y || index % capacity >= min(atomicLoad(&counts[index / capacity]), capacity)) {
    return;
  }

  let base = index * 8u;
  let position = vec3<f32>(pool[base], pool[base + 1u], pool[base + 2u]);
  let distance = length(position - params.camera.xyz);
  let reach = params.rules.y;
  let code = pool[base + 7u];
  let keep = saturate((reach - distance) / (0.2 * reach)) * 1.05;
  let fade = saturate((keep - fract(code)) / 0.05);

  if (fade <= 0.0) {
    return;
  }

  let scale = pool[base + 3u] * fade;
  let centre = position + vec3<f32>(0.0, scale * 0.4, 0.0);

  // Stones under a pixel or so across are not drawn: a stream's cobbles
  // are many.
  if (scale / max(distance, 0.01) * params.rules.w < 0.75) {
    return;
  }

  for (var p = 0; p < 6; p = p + 1) {
    let plane = params.planes[p];

    if (dot(plane.xyz, centre) + plane.w < -scale) {
      return;
    }
  }

  if (atomicAdd(&args[TOTAL], 1u) >= MAX_BOULDERS) {
    return;
  }

  let variant = min(u32(code), VARIANTS - 1u);
  // The finest level while it spans more than 4 % of its distance, the
  // coarsest below 1.2 %.
  let looks = scale / max(distance, 0.01);
  let lod = select(select(2u, 1u, looks > 0.012), 0u, looks > 0.04);
  var starts = array<u32, 3>(0u, 1024u, 3072u);
  var capacities = array<u32, 3>(1024u, 2048u, 4096u);
  append(variant * 3u + lod, variant * VARIANT_ENTRIES + starts[lod], capacities[lod], base, scale);

  if (distance < params.rules.z) {
    append(18u + variant, VARIANTS * VARIANT_ENTRIES + variant * SHADOW_CAPACITY, SHADOW_CAPACITY, base, scale);
  }
}
