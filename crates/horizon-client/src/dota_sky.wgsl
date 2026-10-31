// Dota map sky: the map's cube, tinted for night, faded into the fog colour
// near the horizon as the skybox angular fog of ent_dota_lightinfo does.
struct Sky {
    inverse_view_proj: mat4x4<f32>,
    // rgb tint, w: elevation in radians where the horizon fog has faded out
    tint: vec4<f32>,
    fog_color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> sky: Sky;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var cube: texture_cube<f32>;

struct Vout {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> Vout {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var o: Vout;
    o.clip = vec4<f32>(xy, 0.0, 1.0);
    o.ndc = xy;
    return o;
}

@fragment
fn fs_sky(i: Vout) -> @location(0) vec4<f32> {
    // Reverse Z: depth 1 is the near plane. Two points along the pixel's ray,
    // so the view bob's translation cancels and only the direction is left.
    let near = sky.inverse_view_proj * vec4<f32>(i.ndc, 1.0, 1.0);
    let far = sky.inverse_view_proj * vec4<f32>(i.ndc, 0.001, 1.0);
    let d = normalize(far.xyz / far.w - near.xyz / near.w);
    // Map space (x, y, z) is Source (y, z, x); the cube is sampled in Source space.
    let texel = textureSample(cube, samp, vec3<f32>(d.z, d.x, d.y)).rgb;
    let color = srgb_to_linear(texel) * sky.tint.rgb;
    let elevation = asin(clamp(d.y, -1.0, 1.0));
    let fog = 1.0 - smoothstep(0.0, sky.tint.w, elevation);
    return vec4<f32>(tonemap(mix(color, sky.fog_color.rgb, fog)), 1.0);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// Dota's tonemap curve, as in dota_map.wgsl.
fn tonemap(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0)) * 0.6;
    let mapped = x * (15.8 * x + 2.12) / (x * (1.2 * x + 5.92) + 1.9);
    return linear_to_srgb(clamp(mapped, vec3<f32>(0.0), vec3<f32>(1.0)));
}
