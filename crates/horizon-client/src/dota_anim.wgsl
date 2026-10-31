// Animated Dota map models: skinned by a bone palette the CPU samples each
// frame, lit like dota_map.wgsl's global_lit_simple (linear colour, normal and
// specular maps, the ent_dota_lightinfo global light with the sun shadow, the
// deferred point lights, Dota's tonemap). hero.vfx materials read their
// specular intensity from masks2.r and self illumination from masks1.a.
struct Globals {
    view_proj: mat4x4<f32>,
    shadow_view_proj: mat4x4<f32>,
    // xyz camera, w seconds
    camera: vec4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    ambient_dir: vec4<f32>,
    ambient_color: vec4<f32>,
    sky_vertical: vec4<f32>,
    sky_constant: vec4<f32>,
    specular_dir: vec4<f32>,
    specular_albedo: vec4<f32>,
    specular_lit: vec4<f32>,
    fog_color: vec4<f32>,
    light_grid: vec4<f32>,
    light_grid2: vec4<f32>,
    // x 1 when group 1 holds the real sun shadow map
    flags: vec4<f32>,
}

struct Material {
    kind: u32,
    features: u32,
    mask_scale: f32,
    pad0: f32,
    scales: vec4<f32>,
    offset: vec4<f32>,
    tint: vec4<f32>,
    // x specular intensity, y bump strength, z alpha test reference; [2].xy scroll, z scroll mode, w alpha angle power
    params: array<vec4<f32>, 4>,
}

const KIND_SOLID: u32 = 3u;
const F_NORMAL_MAP: u32 = 1u;
const F_SPECULAR: u32 = 2u;
const F_TRANSLUCENT: u32 = 4u;
const F_ADDITIVE: u32 = 8u;
const F_OVERLAY: u32 = 16u;
const F_ALPHA_TEST: u32 = 32u;
const F_FULLBRIGHT: u32 = 64u;
const F_ALLOW_LIGHTING_ON_TRANSLUCENT: u32 = 128u;
const F_MODULATE_SPECULAR_BY_ALPHA: u32 = 256u;
const F_TINT_MASK: u32 = 512u;
const F_HERO: u32 = 1048576u;

const INCH: f32 = 0.0254;

@group(0) @binding(0) var<uniform> m: Material;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var color_tex: texture_2d<f32>;
@group(0) @binding(3) var scroll_tex: texture_2d<f32>;
@group(0) @binding(4) var mask_tex: texture_2d<f32>;
@group(0) @binding(5) var normal_tex: texture_2d<f32>;
@group(0) @binding(6) var spec_tex: texture_2d<f32>;
@group(0) @binding(7) var masks1_tex: texture_2d<f32>;
@group(1) @binding(0) var shadow_map: texture_depth_2d;
@group(1) @binding(1) var shadow_samp: sampler_comparison;
@group(1) @binding(2) var<storage, read> lights: array<vec4<f32>>;
@group(1) @binding(3) var<storage, read> cells: array<u32>;

@group(1) @binding(5) var ao_vol: texture_3d<f32>;
@group(1) @binding(6) var ao_samp: sampler;
struct AoInfo {
    // minimum corner, cell size
    origin: vec4<f32>,
    // size of the volume in metres, w 1 when there is one
    extent: vec4<f32>,
}
@group(1) @binding(7) var<uniform> ao_info: AoInfo;

// Baked ambient occlusion of the map's solid geometry (dota_map::ao_volume):
// the blurred occupancy around a point lifted off its surface.
fn occlusion(world: vec3<f32>, n: vec3<f32>) -> f32 {
    if (ao_info.extent.w < 0.5) {
        return 0.0;
    }
    let p = (world + n * 0.8 - ao_info.origin.xyz) / ao_info.extent.xyz;
    if (any(p < vec3<f32>(0.0)) || any(p > vec3<f32>(1.0))) {
        return 0.0;
    }
    let occ = textureSampleLevel(ao_vol, ao_samp, p, 0.0).r;
    return clamp((occ - 0.10) * 2.4, 0.0, 0.85);
}
@group(2) @binding(0) var<uniform> g: Globals;
@group(2) @binding(1) var<storage, read> palette: array<mat4x4<f32>>;

struct Vin {
    @location(0) pos: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) tan: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) joints: vec4<u32>,
    @location(5) weights: vec4<f32>,
    @location(6) m0: vec4<f32>,
    @location(7) m1: vec4<f32>,
    @location(8) m2: vec4<f32>,
    @location(9) m3: vec4<f32>,
    @location(10) tint_fade: vec4<f32>,
    @location(11) base: u32,
}

struct Vout {
    @builtin(position) clip: vec4<f32>,
    @location(0) nrm: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) eye: vec3<f32>,
    @location(3) tan: vec4<f32>,
    @location(4) tint_fade: vec4<f32>,
}

@vertex
fn vs(v: Vin) -> Vout {
    var o: Vout;
    o.tint_fade = v.tint_fade;
    if (v.tint_fade.w <= 0.0) {
        o.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
        return o;
    }
    let skin = palette[v.base + v.joints.x] * v.weights.x
        + palette[v.base + v.joints.y] * v.weights.y
        + palette[v.base + v.joints.z] * v.weights.z
        + palette[v.base + v.joints.w] * v.weights.w;
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3) * skin;
    let world = (model * vec4<f32>(v.pos, 1.0)).xyz;
    o.clip = g.view_proj * vec4<f32>(world - g.camera.xyz, 1.0);
    o.nrm = normalize((model * vec4<f32>(v.nrm, 0.0)).xyz + vec3<f32>(1.0e-6));
    o.tan = vec4<f32>(normalize((model * vec4<f32>(v.tan.xyz, 0.0)).xyz + vec3<f32>(1.0e-6)), v.tan.w);
    o.uv = v.uv;
    o.eye = world - g.camera.xyz;
    return o;
}

fn has(f: u32) -> bool {
    return (m.features & f) != 0u;
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

fn tonemap(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0)) * 0.6;
    let mapped = x * (15.8 * x + 2.12) / (x * (1.2 * x + 5.92) + 1.9);
    return linear_to_srgb(clamp(mapped, vec3<f32>(0.0), vec3<f32>(1.0)));
}

fn unpack_normal(texel: vec4<f32>, strength: f32) -> vec3<f32> {
    var n = texel.xyz * 2.0 - 1.0;
    n = vec3<f32>(n.xy * strength, max(n.z, 0.0001));
    return normalize(n);
}

fn to_world(n_ts: vec3<f32>, n: vec3<f32>, t: vec4<f32>) -> vec3<f32> {
    let tangent = normalize(t.xyz - n * dot(n, t.xyz) + vec3<f32>(1.0e-6));
    let bitangent = cross(n, tangent) * select(1.0, -1.0, t.w < 0.0);
    return normalize(tangent * n_ts.x + bitangent * n_ts.y + n * n_ts.z);
}

fn sun_shadow(world: vec3<f32>, n: vec3<f32>) -> f32 {
    if (g.flags.x < 0.5) {
        return 1.0;
    }
    let p = g.shadow_view_proj * vec4<f32>(world + n * 0.06, 1.0);
    let uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || p.z > 1.0) {
        return 1.0;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(shadow_map));
    var lit = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            lit += textureSampleCompareLevel(shadow_map, shadow_samp, uv + vec2<f32>(f32(x), f32(y)) * texel, p.z);
        }
    }
    return lit / 9.0;
}

fn global_light(n: vec3<f32>, shadow: f32, ao: f32) -> vec3<f32> {
    let sun_lit = clamp(dot(n, g.sun_dir.xyz), 0.0, 1.0) * shadow;
    let open = 1.0 - ao;
    let sun = g.sun_color.rgb * sun_lit * (0.45 + 0.55 * pow(open, 1.85));
    let dark = pow(open, 2.1);
    let ambient_shadow = mix(1.0, shadow, g.ambient_dir.w);
    let directional = g.ambient_color.rgb * clamp(dot(n, g.ambient_dir.xyz), 0.0, 1.0) * ambient_shadow * dark;
    let fill = (g.sky_vertical.rgb * n.y + g.sky_constant.rgb) * (1.0 - sun_lit) * dark;
    return sun + directional + fill;
}

fn point_lights(world: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    if (g.light_grid.z <= 0.0) {
        return vec3<f32>(0.0);
    }
    let col = i32(floor((world.x - g.light_grid.x) / g.light_grid.z));
    let row = i32(floor((world.z - g.light_grid.y) / g.light_grid.z));
    let cols = i32(g.light_grid.w);
    let rows = i32(g.light_grid2.x);
    if (col < 0 || row < 0 || col >= cols || row >= rows) {
        return vec3<f32>(0.0);
    }
    let at = u32(row * cols + col) * (u32(g.light_grid2.y) + 1u);
    let count = cells[at];
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < count; i++) {
        let index = cells[at + 1u + i];
        let p = lights[index * 2u];
        let color = lights[index * 2u + 1u].rgb;
        let to_light = p.xyz - world;
        let d = length(to_light);
        let t = clamp((1.0 - d / max(p.w, 0.001)) / 0.5, 0.0, 1.0);
        let facing = max(dot(n, to_light / max(d, 0.001)), 0.0);
        sum += color * (t * t * (3.0 - 2.0 * t)) * facing;
    }
    return sum;
}

const FOG_START: f32 = 25.0;
const FOG_END: f32 = 140.0;

fn distance_fog(eye: vec3<f32>) -> f32 {
    let t = clamp((length(eye) - FOG_START) / (FOG_END - FOG_START), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn illumination(world: vec3<f32>, n: vec3<f32>, shadow: f32) -> vec3<f32> {
    return global_light(n, shadow, occlusion(world, n)) + point_lights(world, n);
}

fn dota_specular(world: vec3<f32>, n: vec3<f32>) -> f32 {
    let to_camera = normalize(g.camera.xyz - world);
    let facing = clamp(dot(g.specular_dir.xyz, reflect(to_camera, n)), 0.0, 1.0);
    return pow(facing, g.specular_dir.w);
}

// 4x4 ordered dither, so day/night fades keep depth writes and need no sorting.
fn bayer(p: vec2<f32>) -> f32 {
    let x = u32(p.x) & 3u;
    let y = u32(p.y) & 3u;
    var table = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    return (table[y * 4u + x] + 0.5) / 16.0;
}

@fragment
fn fs(i: Vout, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let blended = has(F_TRANSLUCENT) || has(F_ADDITIVE);
    let fade = i.tint_fade.w;
    let world = i.eye + g.camera.xyz;
    var vn = normalize(i.nrm);
    if (!front) {
        vn = -vn;
    }
    let time = g.camera.w;
    var uv = i.uv * m.scales.xy + m.offset.xy;
    var uv_color = uv;
    var scroll = vec2<f32>(1.0);
    if (m.params[2].z > 0.5) {
        scroll = textureSample(scroll_tex, samp, uv).ga;
        uv_color = uv + m.params[2].xy * time * scroll.x;
        if (m.params[2].z > 1.5) {
            uv = uv_color;
        }
    }
    let texel = textureSample(color_tex, samp, uv_color);
    let normal_texel = textureSample(normal_tex, samp, uv);
    let spec_texel = textureSample(spec_tex, samp, uv);
    let mask = textureSample(mask_tex, samp, uv);
    let masks1 = textureSample(masks1_tex, samp, uv);
    var tint = srgb_to_linear(m.tint.rgb * i.tint_fade.rgb);
    if (has(F_TINT_MASK)) {
        tint = mix(vec3<f32>(1.0), tint, mask.g);
    }
    var albedo: vec3<f32>;
    var alpha = 1.0;
    if (m.kind == KIND_SOLID) {
        albedo = tint;
    } else {
        albedo = srgb_to_linear(texel.rgb) * tint;
        alpha = texel.a;
        if (has(F_TRANSLUCENT) && m.params[2].z > 0.5) {
            alpha = scroll.y;
        }
        alpha *= m.mask_scale;
    }
    if (m.params[2].w > 0.0) {
        alpha *= pow(abs(dot(normalize(g.camera.xyz - world), vn)), m.params[2].w);
    }
    if (!blended && fade < 1.0 && fade <= bayer(i.clip.xy)) {
        discard;
    }
    if (has(F_ALPHA_TEST) && alpha < m.params[0].z) {
        discard;
    }
    if (!has(F_TRANSLUCENT)) {
        alpha = 1.0;
    }
    var n = vn;
    if (has(F_NORMAL_MAP)) {
        n = to_world(unpack_normal(normal_texel, m.params[0].y), vn, i.tan);
    }
    var spec = vec2<f32>(0.0);
    if (has(F_HERO)) {
        spec = vec2<f32>(spec_texel.r * m.params[0].x, masks1.a);
    } else if (has(F_SPECULAR)) {
        spec = vec2<f32>(spec_texel.x * m.params[0].x, spec_texel.y);
        if (has(F_MODULATE_SPECULAR_BY_ALPHA)) {
            spec.x *= alpha;
        }
    }
    let unlit = has(F_FULLBRIGHT)
        || (has(F_TRANSLUCENT) && !has(F_ALLOW_LIGHTING_ON_TRANSLUCENT) && !has(F_OVERLAY));
    var lit: vec3<f32>;
    if (unlit) {
        lit = albedo;
    } else {
        let shadow = sun_shadow(world, vn);
        let light = illumination(world, n, shadow);
        let specular = dota_specular(world, n) * spec.x;
        let specular_albedo = albedo + specular * g.specular_albedo.rgb;
        lit = light * specular_albedo + specular * g.specular_lit.rgb * shadow;
        lit = mix(lit, specular_albedo, spec.y);
    }
    let haze = distance_fog(i.eye);
    lit = mix(lit, g.fog_color.rgb, haze);
    if (has(F_ADDITIVE)) {
        let x = max(lit * g.sun_dir.w, vec3<f32>(0.0)) * 0.6;
        let mapped = clamp(x * (15.8 * x + 2.12) / (x * (1.2 * x + 5.92) + 1.9), vec3<f32>(0.0), vec3<f32>(1.0));
        let a = alpha * fade;
        return vec4<f32>(mapped * a * (1.0 - haze), a);
    }
    return vec4<f32>(tonemap(lit * g.sun_dir.w), alpha * select(1.0, fade, blended));
}
