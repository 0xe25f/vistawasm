// Bakes terrain self-shadowing for the current sun direction.
//
// For each output texel, march across the height field towards the sun
// and track the steepest horizon angle seen. The point is lit when the sun
// is above that horizon, with a soft penumbra of adjustable width. Steps
// grow geometrically, so a 1024-texel march reaches the far side of the
// map in under 100 loads. Run only when the sun or terrain changes.

struct BakeParams {
  // xyz: unit vector towards the sun, w: penumbra softness (0 to 1).
  sun: vec4<f32>,
  // x: metres per height texel, y: height texels per output texel,
  // zw: height texture size.
  grid: vec4<f32>,
};

@group(0) @binding(0) var heights: texture_2d<f32>;
@group(0) @binding(1) var output: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: BakeParams;

fn height_at(texel: vec2<f32>) -> f32 {
  let size = params.grid.zw;
  let clamped = clamp(texel, vec2<f32>(0.0), size - vec2<f32>(1.001));
  let base = floor(clamped);
  let f = clamped - base;
  let i = vec2<i32>(base);
  let h00 = textureLoad(heights, i, 0).r;
  let h10 = textureLoad(heights, i + vec2<i32>(1, 0), 0).r;
  let h01 = textureLoad(heights, i + vec2<i32>(0, 1), 0).r;
  let h11 = textureLoad(heights, i + vec2<i32>(1, 1), 0).r;
  return mix(mix(h00, h10, f.x), mix(h01, h11, f.x), f.y);
}

@compute @workgroup_size(8, 8, 1)
fn bake(@builtin(global_invocation_id) id: vec3<u32>) {
  let out_size = textureDimensions(output);

  if (id.x >= out_size.x || id.y >= out_size.y) {
    return;
  }

  let sun = normalize(params.sun.xyz);
  var lit = 0.0;

  if (sun.y > 0.0) {
    let origin = (vec2<f32>(id.xy) + 0.5) * params.grid.y;
    let size = params.grid.zw;
    let h0 = height_at(origin) + 0.5;
    let flat_length = length(sun.xz);
    let sun_slope = sun.y / max(flat_length, 0.0001);
    lit = 1.0;

    if (flat_length > 0.001) {
      let direction = sun.xz / flat_length;
      let metres = params.grid.x;
      var travelled = 1.0;
      var step = 1.0;
      var horizon = -1.0e6;

      for (var i = 0; i < 96; i = i + 1) {
        let p = origin + direction * travelled;

        if (any(p < vec2<f32>(0.0)) || any(p > size - 1.0)) {
          break;
        }

        horizon = max(horizon, (height_at(p) - h0) / (travelled * metres));
        travelled = travelled + step;
        step = step * 1.06;
      }

      // Penumbra: the sun has an angular size, and softness widens it.
      let penumbra = 0.01 + params.sun.w * 0.12;
      lit = smoothstep(-penumbra, penumbra, sun_slope - horizon);
    }
  }

  textureStore(output, vec2<i32>(id.xy), vec4<f32>(lit, lit, lit, 1.0));
}
