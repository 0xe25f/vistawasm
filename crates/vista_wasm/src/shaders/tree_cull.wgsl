// GPU tree culling and level-of-detail selection.
//
// One invocation per tree instance: frustum-cull its bounding sphere, drop
// trees smaller than a pixel, then append it to the full-mesh list, the
// impostor list, or both (inside the cross-fade band) for its species.
// Appending bumps the instance count inside the indirect draw arguments,
// so the CPU never reads anything back and never re-uploads instances.
//
// Trees within the shadow radius are also appended to a shadow-caster list
// regardless of the camera frustum, so trees just off screen still cast
// shadows into view.

struct TreeInstance {
  position: vec3<f32>,
  scale: f32,
  rotation: f32,
  tint: f32,
  species: u32,
  dryness: f32,
};

struct OutInstance {
  position: vec3<f32>,
  scale: f32,
  rotation: f32,
  tint: f32,
  species_fade: f32,
  dryness: f32,
};

struct CullParams {
  planes: array<vec4<f32>, 6>,
  // xyz: camera position, w: mesh distance.
  camera: vec4<f32>,
  // x: max distance, y: tree style (0 billboard, 1 cross, 2 mesh),
  // z: pixels per radian, w: instance count.
  params: vec4<f32>,
  // Per species: x height, y radius.
  bounds: array<vec4<f32>, 8>,
  // Per species first output slot, packed four per vector.
  offsets: array<vec4<u32>, 2>,
  // xy: shadow area centre (x, z), z: radius, w: 1 when trees cast shadows.
  shadow: vec4<f32>,
};

@group(0) @binding(0) var<uniform> cull: CullParams;
@group(0) @binding(1) var<storage, read> instances: array<TreeInstance>;
@group(0) @binding(2) var<storage, read_write> mesh_out: array<OutInstance>;
@group(0) @binding(3) var<storage, read_write> impostor_out: array<OutInstance>;
@group(0) @binding(4) var<storage, read_write> args: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> shadow_out: array<OutInstance>;

// Mesh draws use indexed indirect arguments (5 words each) starting at
// word 0; impostor draws use non-indexed arguments (4 words each)
// starting at word 40.
const IMPOSTOR_ARGS_BASE: u32 = 40u;
// Shadow-caster draws: non-indexed, 4 words each, from word 72.
const SHADOW_ARGS_BASE: u32 = 72u;

fn species_offset(species: u32) -> u32 {
  let packed = cull.offsets[species / 4u];
  return packed[species % 4u];
}

fn pack(instance: TreeInstance, fade: f32) -> OutInstance {
  var out: OutInstance;
  out.position = instance.position;
  out.scale = instance.scale;
  out.rotation = instance.rotation;
  out.tint = instance.tint;
  out.species_fade = f32(instance.species) + min(fade, 0.998) * 0.5;
  out.dryness = instance.dryness;
  return out;
}

fn emit(instance: TreeInstance, fade: f32, to_mesh: bool) {
  let out = pack(instance, fade);

  if (to_mesh) {
    let slot = atomicAdd(&args[instance.species * 5u + 1u], 1u);
    mesh_out[species_offset(instance.species) + slot] = out;
  } else {
    let slot = atomicAdd(&args[IMPOSTOR_ARGS_BASE + instance.species * 4u + 1u], 1u);
    impostor_out[species_offset(instance.species) + slot] = out;
  }
}

@compute @workgroup_size(64)
fn cull_main(@builtin(global_invocation_id) id: vec3<u32>) {
  let index = id.x;

  if (index >= u32(cull.params.w)) {
    return;
  }

  var instance = instances[index];
  let species = min(instance.species, 7u);
  instance.species = species;
  let bounds = cull.bounds[species];
  let height = bounds.x * instance.scale;
  let radius = max(bounds.y * instance.scale, height * 0.5);
  let centre = instance.position + vec3<f32>(0.0, height * 0.5, 0.0);

  if (cull.shadow.w > 0.5 && distance(instance.position.xz, cull.shadow.xy) < cull.shadow.z + radius) {
    let slot = atomicAdd(&args[SHADOW_ARGS_BASE + species * 4u + 1u], 1u);
    shadow_out[species_offset(species) + slot] = pack(instance, 0.0);
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
    emit(instance, 0.0, false);
    return;
  }

  let mesh_distance = cull.camera.w * (0.75 + instance.scale * 0.25);
  let band = mesh_distance * 0.18;

  if (distance < mesh_distance - band) {
    emit(instance, 1.0, true);
  } else if (distance < mesh_distance) {
    // Cross-fade band: draw both with complementary dithers.
    let fade = saturate((mesh_distance - distance) / band);
    emit(instance, fade, true);
    emit(instance, fade, false);
  } else {
    emit(instance, 0.0, false);
  }
}
