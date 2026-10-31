// Dota's world particles as sprites (dota_fx.rs). One instance per particle,
// six vertices per quad. Sizes follow particles.dll: the radius is clamped to
// min..max size of the screen and faded out between the fade sizes.

struct U {
    view_proj: mat4x4<f32>,
    camera: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    // xyz forward, w focal length (NDC units of a 1 m object at 1 m)
    forward: vec4<f32>,
    // exposure, fog start, fog end
    misc: vec4<f32>,
    fog_color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct In {
    @location(0) pos_radius: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec4<f32>,
    // roll, min size, max size, add self
    @location(3) params: vec4<f32>,
    // fade start, fade end, flat, yaw
    @location(4) fade: vec4<f32>,
}

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) add_self: f32,
    @location(3) dist: f32,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, i: In) -> Out {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = corners[vi];
    var o: Out;
    let center = i.pos_radius.xyz;
    let to = center - u.camera.xyz;
    let dist = dot(to, u.forward.xyz);
    o.dist = length(to);
    if (dist < 0.05) {
        o.clip = vec4<f32>(0.0, 0.0, -2.0, 1.0);
        return o;
    }
    let focal = u.forward.w;
    let size = i.pos_radius.w * focal / dist;
    let clamped = clamp(size, i.params.y, i.params.z);
    let fade = 1.0 - smoothstep(i.fade.x, i.fade.y, clamped);
    let r = clamped * dist / focal;
    let roll = i.params.x;
    let cs = cos(roll);
    let sn = sin(roll);
    let q = vec2<f32>(c.x * cs - c.y * sn, c.x * sn + c.y * cs);
    var world: vec3<f32>;
    if (i.fade.z > 0.5) {
        // World Z aligned: a card lying flat (Source Z is map Y).
        let yaw = i.fade.w;
        let ys = sin(yaw);
        let yc = cos(yaw);
        let f = vec2<f32>(q.x * yc - q.y * ys, q.x * ys + q.y * yc);
        world = center + vec3<f32>(f.x, 0.0, f.y) * i.pos_radius.w;
    } else {
        world = center + (u.right.xyz * q.x + u.up.xyz * q.y) * r;
    }
    // view_proj is camera-relative, like the map's.
    o.clip = u.view_proj * vec4<f32>(world - u.camera.xyz, 1.0);
    let t = c * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    o.uv = mix(i.uv.xy, i.uv.zw, t);
    o.color = vec4<f32>(i.color.rgb, i.color.a * fade);
    o.add_self = i.params.w;
    return o;
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

// Dota's tonemap curve, before the sRGB encode.
fn curve(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0)) * 0.6;
    return clamp(x * (15.8 * x + 2.12) / (x * (1.2 * x + 5.92) + 1.9), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn haze(dist: f32) -> f32 {
    let t = clamp((dist - u.misc.y) / (u.misc.z - u.misc.y), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Alpha blended, premultiplied: m_flAddSelfAmount 0 is plain alpha blending, 1
// adds the sprite on top of what is behind instead of covering it.
@fragment
fn fs_alpha(i: Out) -> @location(0) vec4<f32> {
    let texel = textureSample(tex, samp, i.uv);
    let a = clamp(texel.a * i.color.a, 0.0, 1.0);
    let hdr = srgb_to_linear(texel.rgb) * i.color.rgb * u.misc.x;
    let h = haze(i.dist);
    let rgb = mix(linear_to_srgb(curve(hdr)), u.fog_color.rgb, h);
    let self_add = clamp(i.add_self, 0.0, 1.0);
    let boost = 1.0 + max(i.add_self - 1.0, 0.0);
    return vec4<f32>(rgb * a * boost, a * (1.0 - self_add));
}

// Additive: added in the tonemapped linear range, as the map's additive batches
// do; sRGB-encoding a small addition would wash the frame out.
@fragment
fn fs_add(i: Out) -> @location(0) vec4<f32> {
    let texel = textureSample(tex, samp, i.uv);
    let a = clamp(texel.a * i.color.a, 0.0, 1.0);
    let hdr = srgb_to_linear(texel.rgb) * i.color.rgb * u.misc.x;
    return vec4<f32>(curve(hdr) * a * (1.0 - haze(i.dist)), 0.0);
}

// Modulate 2x: grey leaves the frame alone, darker darkens, lighter brightens.
@fragment
fn fs_mod2x(i: Out) -> @location(0) vec4<f32> {
    let texel = textureSample(tex, samp, i.uv);
    let a = clamp(texel.a * i.color.a, 0.0, 1.0) * (1.0 - haze(i.dist));
    return vec4<f32>(mix(vec3<f32>(0.5), texel.rgb, a), 0.0);
}
