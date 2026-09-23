// Builds one mip level of an rgba8 texture array from the level above.
//
// Colour is averaged weighted by alpha so transparent texels (the empty
// space around leaves and impostors) do not bleed dark fringes into the
// smaller mips, and alpha is boosted slightly so alpha-tested foliage keeps
// its coverage in the distance instead of thinning into nothing. Opaque
// textures (terrain, bark) are unaffected by either adjustment.
//
// `mode`: 0 = sRGB-encoded opaque colour, 1 = sRGB-encoded colour with
// alpha coverage, 2 = linear data (normals, roughness, noise).

struct MipParams {
  mode: u32,
  pad0: u32,
  pad1: u32,
  pad2: u32,
};

@group(0) @binding(0) var source: texture_2d_array<f32>;
@group(0) @binding(1) var destination: texture_storage_2d_array<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: MipParams;

@compute @workgroup_size(8, 8, 1)
fn downsample(@builtin(global_invocation_id) id: vec3<u32>) {
  let size = textureDimensions(destination);

  if (id.x >= size.x || id.y >= size.y) {
    return;
  }

  let base = vec2<i32>(id.xy) * 2;
  let layer = i32(id.z);
  var colour = vec3<f32>(0.0);
  var plain = vec4<f32>(0.0);
  var alpha = 0.0;

  for (var y = 0; y < 2; y = y + 1) {
    for (var x = 0; x < 2; x = x + 1) {
      let texel = textureLoad(source, base + vec2<i32>(x, y), layer, 0);
      // Average in linear light so mips keep the right brightness.
      let linear = select(pow(texel.rgb, vec3<f32>(2.2)), texel.rgb, params.mode == 2u);
      colour = colour + linear * texel.a;
      plain = plain + vec4<f32>(linear, texel.a);
      alpha = alpha + texel.a;
    }
  }

  var result: vec4<f32>;

  if (params.mode == 1u) {
    let rgb = select(plain.rgb / 4.0, colour / max(alpha, 0.0001), alpha > 0.0001);
    result = vec4<f32>(pow(rgb, vec3<f32>(1.0 / 2.2)), min(alpha / 4.0 * 1.18, 1.0));
  } else if (params.mode == 2u) {
    result = plain / 4.0;
  } else {
    result = vec4<f32>(pow(plain.rgb / 4.0, vec3<f32>(1.0 / 2.2)), plain.a / 4.0);
  }

  textureStore(destination, vec2<i32>(id.xy), layer, result);
}
