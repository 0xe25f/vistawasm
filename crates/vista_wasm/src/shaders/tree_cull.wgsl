// GPU tree culling and level-of-detail selection. `lattice.wgsl` comes
// first.
//
// One invocation per tree: the static far set (or the host's trees)
// first, then every entry of the streamed tile pool, whose slots hold
// their own counts. Lattice trees are thinned with distance by their
// rank (`thinning`), and the ones kept grow so the canopy stays closed;
// between 0.8 and 1 times the canopy distance they give way, by hash, to
// the canopy layer. Each tree is then frustum-culled by its bounding
// sphere, trees smaller than a pixel are dropped, and the rest go to a
// mesh list or the impostor list. Appending bumps the instance count
// inside the indirect draw arguments, so the CPU never waits for
// anything.
//
// There is one mesh list per species, variant, age class and level of
// detail (`tree_models::mesh_slot`), then one per species and variant
// for understorey saplings. Near trees draw the full mesh (LOD0), trees
// beyond `cull.lod.x` the lighter one (LOD1), and trees beyond the mesh
// distance an impostor. Across the last tenth of each band a tree draws
// both, with complementary dithers, so nothing pops. A full list passes
// its trees on to the lighter level, and the last to impostors.
//
// Understorey saplings beyond `cull.budget.z` (15 m) are impostor cards:
// under a closed canopy they are half shaded and seen through foliage.
// The engine reads the draw counts back a frame late to hold the tree
// triangles to their budget, bringing the mesh distance in (the furthest
// meshes become impostors first) and the shadow casters' reach.
//
// Trees within the shadow radius, and within the casters' reach across
// the ground from the camera (the shadow distance, or less when the
// triangle budget brings it in), are also appended to the shadow-caster
// list regardless of the camera frustum, so trees just off screen still
// cast shadows into view. The list holds at most `cull.budget.w` casters.
//
// Trees read here already stand on the drawn terrain: `grounding.wgsl`
// grounds them each time the mesh recentres, and the generators when
// they fill a tile.
//
// Drawn trees carry `species | variant << 3 | lean << 5` plus half their
// fade in one float, negated where the draw takes the complement of the
// dither (`trees.wgsl`).

struct TreeInstance {
  position: vec3<f32>,
  scale: f32,
  rotation: f32,
  tint: f32,
  // Bits 0 to 7: species; the rest are flags and the lattice rank (see
  // `render/flora.rs`).
  species: u32,
  dryness: f32,
};

struct CullParams {
  planes: array<vec4<f32>, 6>,
  // xyz: camera position, w: mesh distance.
  camera: vec4<f32>,
  // x: max distance, y: tree style (0 billboard, 1 cross, 2 mesh),
  // z: pixels per radian, w: instances to cull.
  params: vec4<f32>,
  // Per species: x height, y radius.
  bounds: array<vec4<f32>, 8>,
  // x: where the full meshes give way to the lighter ones, y: unused, z:
  // variants with meshes, w: variants with impostors.
  lod: vec4<f32>,
  // xy: shadow area centre (x, z), z: radius, w: 1 when trees cast shadows.
  shadow: vec4<f32>,
  // x: instances before the tile pool, y: instances per tile slot,
  // zw: unused.
  stream: vec4<u32>,
  // x: full-density radius, y: far keep fraction, z: canopy distance,
  // w: the understorey's first rank relative to p.
  thin: vec4<f32>,
  // x: shadow casters' reach across the ground from the camera, y:
  // unused, z: understorey card distance, w: most shadow casters.
  budget: vec4<f32>,
};

@group(0) @binding(0) var<uniform> cull: CullParams;
@group(0) @binding(1) var<storage, read> instances: array<TreeInstance>;
// Drawn trees, 9 floats each: position, scale, rotation, tint, species
// plus fade / 2, dryness and extra crown width.
@group(0) @binding(2) var<storage, read_write> mesh_out: array<f32>;
@group(0) @binding(3) var<storage, read_write> impostor_out: array<f32>;
@group(0) @binding(4) var<storage, read_write> args: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> shadow_out: array<f32>;
@group(0) @binding(6) var<storage, read_write> counts: array<atomic<u32>>;
// Per mesh list: its first slot in `mesh_out`, and how many it holds.
@group(0) @binding(7) var<storage, read> lists: array<vec2<u32>>;

// Mesh draws use indexed indirect arguments (5 words each) from word 0,
// one per mesh list; the impostor draw non-indexed arguments (4 words)
// after them, and the shadow-caster draw after that.
const CANOPY_LISTS: u32 = 256u;
const MESH_LISTS: u32 = 288u;
const IMPOSTOR_ARGS: u32 = 1440u;
const SHADOW_ARGS: u32 = 1444u;
// `tree_growth::{VARIANTS, AGES, LODS}` and `render/flora.rs`.
const VARIANTS: u32 = 4u;
const AGES: u32 = 4u;
const VARIANT_SHIFT: u32 = 10u;
const AGE_SHIFT: u32 = 12u;
const LEAN_SHIFT: u32 = 14u;
// `TREE_LATTICE` and `TREE_RANK_SHIFT` in `render/flora.rs`.
const LATTICE: u32 = 1048576u;
const RANK_SHIFT: u32 = 21u;
const UNDERSTOREY_SHADOW_METRES: f32 = 60.0;

// `value` is the drawn code plus half the fade, negated for the
// complementary dither.
fn write(list: u32, slot: u32, instance: TreeInstance, value: f32, width: f32) {
  let base = slot * 9u;
  let values = array<f32, 9>(
    instance.position.x,
    instance.position.y,
    instance.position.z,
    instance.scale,
    instance.rotation,
    instance.tint,
    value,
    instance.dryness,
    width,
  );

  for (var i = 0u; i < 9u; i = i + 1u) {
    if (list == 0u) {
      mesh_out[base + i] = values[i];
    } else if (list == 1u) {
      impostor_out[base + i] = values[i];
    } else {
      shadow_out[base + i] = values[i];
    }
  }
}

// The drawn value: `code` plus half of `fade`, negated to draw where the
// dither is above `fade` rather than at or below it.
fn drawn(code: u32, fade: f32, complement: bool) -> f32 {
  let value = f32(code) + clamp(fade, 0.002, 0.998) * 0.5;
  return select(value, -value, complement);
}

// Append to a mesh list; false when it is full.
fn emit_mesh(instance: TreeInstance, value: f32, width: f32, list: u32) -> bool {
  let word = list * 5u + 1u;
  let slot = atomicAdd(&args[word], 1u);
  let room = lists[list];

  if (slot < room.y) {
    write(0u, room.x + slot, instance, value, width);
    return true;
  }

  atomicSub(&args[word], 1u);
  return false;
}

fn emit_impostor(instance: TreeInstance, value: f32, width: f32) {
  write(1u, atomicAdd(&args[IMPOSTOR_ARGS + 1u], 1u), instance, value, width);
}

// A mesh at `list` (a LOD0 list when `light` is the matching LOD1 list),
// falling back to the lighter mesh and then an impostor, drawn where the
// mesh would have been, when full.
fn emit(instance: TreeInstance, code: u32, fade: f32, complement: bool, width: f32, list: u32, light: u32) {
  let value = drawn(code, fade, complement);

  if (!emit_mesh(instance, value, width, list)
    && (light == list || !emit_mesh(instance, value, width, light))) {
    emit_impostor(instance, value, width);
  }
}

@compute @workgroup_size(64)
fn cull_main(@builtin(global_invocation_id) id: vec3<u32>) {
  let index = id.x;

  if (index >= u32(cull.params.w)) {
    return;
  }

  // Tile pool entries past their slot's count are empty.
  if (index >= cull.stream.x) {
    let entry = index - cull.stream.x;

    if (entry % cull.stream.y >= min(atomicLoad(&counts[entry / cull.stream.y]), cull.stream.y)) {
      return;
    }
  }

  var instance = instances[index];
  let word = instance.species;
  let species = min(word & 255u, 7u);
  let variant = (word >> VARIANT_SHIFT) & 3u;
  let age = (word >> AGE_SHIFT) & 3u;
  let lean = (word >> LEAN_SHIFT) & 63u;
  let mesh_variant = variant % max(u32(cull.lod.z), 1u);
  // The impostors of variants not baked yet show the first.
  let code = species | ((variant % max(u32(cull.lod.w), 1u)) << 3u) | (lean << 5u);
  instance.species = species;
  var width = 1.0;
  var casts = true;
  var understorey = false;

  if ((word & LATTICE) != 0u) {
    let distance = length(instance.position - cull.camera.xyz);
    let rank = f32(word >> RANK_SHIFT) / 2047.0;
    let t = thinning(distance, cull.thin.x, cull.thin.y);
    let size = thinning_fade(t, rank);
    // Understorey saplings stand in the canopy's shade: only those near
    // the camera, in clearings as likely as not, cast shadows.
    understorey = rank >= cull.thin.w;
    casts = !understorey || distance < UNDERSTOREY_SHADOW_METRES;
    // Past 0.8 of the canopy distance the canopy layer takes over, tree
    // by tree.
    let canopy = cull.thin.z;
    let hash = unit(pcg3d(bitcast<vec3<u32>>(instance.position)).x);

    if (size <= 0.0 || hash >= (canopy - distance) / (0.2 * canopy)) {
      return;
    }

    instance.scale = instance.scale * size * t.z;
    width = t.w;
  }

  let bounds = cull.bounds[species];
  let height = bounds.x * instance.scale;
  let radius = max(bounds.y * instance.scale * width, height * 0.5);
  let centre = instance.position + vec3<f32>(0.0, height * 0.5, 0.0);

  if (casts
    && cull.shadow.w > 0.5
    && distance(instance.position.xz, cull.shadow.xy) < cull.shadow.z + radius
    && distance(instance.position.xz, cull.camera.xz) < cull.budget.x + radius) {
    let slot = atomicAdd(&args[SHADOW_ARGS + 1u], 1u);

    if (f32(slot) < cull.budget.w) {
      write(2u, slot, instance, f32(code), width);
    } else {
      atomicSub(&args[SHADOW_ARGS + 1u], 1u);
    }
  }

  for (var p = 0; p < 6; p = p + 1) {
    let plane = cull.planes[p];

    if (dot(plane.xyz, centre) + plane.w < -radius) {
      return;
    }
  }

  let distance = length(centre - cull.camera.xyz);

  if (distance > cull.params.x) {
    return;
  }

  // Skip trees that would cover less than about one pixel.
  if (radius / max(distance, 0.001) * cull.params.z < 0.6) {
    return;
  }

  let style = cull.params.y;

  if (style < 1.5) {
    emit_impostor(instance, drawn(code, 0.0, true), width);
    return;
  }

  let reach = 0.75 + instance.scale * 0.25;
  var mesh_distance = cull.camera.w * reach;

  if (understorey) {
    // Saplings are young meshes, then cards.
    mesh_distance = min(mesh_distance, cull.budget.z);
    let list = CANOPY_LISTS + species * VARIANTS + mesh_variant;
    let band = mesh_distance * 0.1;

    if (distance < mesh_distance - band) {
      emit(instance, code, 1.0, false, width, list, list);
    } else if (distance < mesh_distance) {
      let fade = saturate((mesh_distance - distance) / band);

      emit(instance, code, fade, false, width, list, list);
      emit_impostor(instance, drawn(code, fade, true), width);
    } else {
      emit_impostor(instance, drawn(code, 0.0, true), width);
    }

    return;
  }

  // `tree_models::mesh_slot`.
  let full = ((species * VARIANTS + mesh_variant) * AGES + age) * 2u;
  let light = full + 1u;
  let lod0 = min(cull.lod.x * reach, mesh_distance);
  let band0 = lod0 * 0.1;
  let band = mesh_distance * 0.1;

  if (distance < lod0 - band0) {
    emit(instance, code, 1.0, false, width, full, light);
  } else if (distance < lod0) {
    // Full and light meshes cross-fade.
    let fade = saturate((lod0 - distance) / band0);

    emit(instance, code, fade, false, width, full, light);
    emit(instance, code, fade, true, width, light, light);
  } else if (distance < mesh_distance - band) {
    emit(instance, code, 1.0, false, width, light, light);
  } else if (distance < mesh_distance) {
    // Light meshes and impostors cross-fade.
    let fade = saturate((mesh_distance - distance) / band);

    emit(instance, code, fade, false, width, light, light);
    emit_impostor(instance, drawn(code, fade, true), width);
  } else {
    emit_impostor(instance, drawn(code, 0.0, true), width);
  }
}
