// Dota map layer, shaded as Dota's multiblend, global_lit_simple and
// water_dota do (after the reverse engineered shaders of ValveResourceFormat):
// linear colour, normal and specular maps, the global light of
// ent_dota_lightinfo with the sun shadow, the deferred point lights, and the
// game's tonemap curve.
struct Uniforms {
    view_proj: mat4x4<f32>,
    camera: vec3<f32>,
    time: f32,
    kind: u32,
    features: u32,
    mask_scale: f32,
    pad0: f32,
    scales: vec4<f32>,
    rotate: vec4<f32>,
    tints: array<vec4<f32>, 4>,
    offset: array<vec4<f32>, 4>,
    tint_b: array<vec4<f32>, 4>,
    // lit: x specular intensity, y bump strength, z alpha test reference; [2].xy scroll
    // multiblend: x specular intensity, y bump strength
    // water: [0] surface colour, w use fog colour; [1] reflection colour; [2] flow xy, time scale, normal uv scale; [3] fog colour
    params: array<vec4<f32>, 4>,
    // xyz towards the sun, w the exposure
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    // xyz the direction the directional ambient faces, w how much the sun shadow darkens it
    ambient_dir: vec4<f32>,
    ambient_color: vec4<f32>,
    // sky and ground fill: vertical * n.y + constant
    sky_vertical: vec4<f32>,
    sky_constant: vec4<f32>,
    // xyz the direction the specular light travels, w its exponent
    specular_dir: vec4<f32>,
    specular_albedo: vec4<f32>,
    specular_lit: vec4<f32>,
    fog_color: vec4<f32>,
    light_grid: vec4<f32>,
    light_grid2: vec4<f32>,
    // Cloud shadows of env_global_light: xy, zw each layer's scroll (uv per second)
    clouds: vec4<f32>,
    // x: 1 / cloudscale (per inch), y: 1 when there are clouds
    clouds2: vec4<f32>,
    // The lightinfo's height fog: linear colour, density (night only on this map)
    height_fog: vec4<f32>,
    shadow_view_proj: mat4x4<f32>,
}

const KIND_LIT: u32 = 0u;
const KIND_MULTIBLEND: u32 = 1u;
const KIND_WATER: u32 = 2u;
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
const F_ENABLE_WIND: u32 = 8192u;
const F_HERO: u32 = 1048576u;
const F_CRYSTAL: u32 = 2097152u;
// g_vWindDir, g_vWindOffset and g_vWindStrengthFreqMulHighStrength defaults.
const WIND_DIR: vec2<f32> = vec2<f32>(0.707, 0.707);
const WIND_OFFSET: vec3<f32> = vec3<f32>(0.25, 0.2, 0.21);
const WIND_STRENGTH: vec4<f32> = vec4<f32>(0.25, 0.03, 0.003, 0.25);

// Map space is Source inches turned Y-up and shifted, see bake_dota_map.py.
const INCH: f32 = 0.0254;
const MAP_OFFSET: vec3<f32> = vec3<f32>(-56.48959, 193.49749, -0.270934);

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var color0: texture_2d<f32>;
@group(0) @binding(3) var color1: texture_2d<f32>;
@group(0) @binding(4) var color2: texture_2d<f32>;
@group(0) @binding(5) var color3: texture_2d<f32>;
@group(0) @binding(6) var mask_tex: texture_2d<f32>;
@group(0) @binding(7) var normal0: texture_2d<f32>;
@group(0) @binding(8) var normal1: texture_2d<f32>;
@group(0) @binding(9) var normal2: texture_2d<f32>;
@group(0) @binding(10) var normal3: texture_2d<f32>;
@group(0) @binding(11) var spec0: texture_2d<f32>;
@group(0) @binding(12) var spec1: texture_2d<f32>;
@group(0) @binding(13) var spec2: texture_2d<f32>;
@group(0) @binding(14) var spec3: texture_2d<f32>;
@group(1) @binding(0) var shadow_map: texture_depth_2d;
@group(1) @binding(1) var shadow_samp: sampler_comparison;
@group(1) @binding(4) var cloud_tex: texture_2d<f32>;
// pairs: (position, radius), (colour * intensity, 0)
@group(1) @binding(2) var<storage, read> lights: array<vec4<f32>>;
// per cell: a count, then that many light indices
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

struct Vin {
    @location(0) pos: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) tan: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) blend: vec3<f32>,
    @location(5) soft: vec3<f32>,
    @location(6) color: vec3<f32>,
    @location(7) m0: vec4<f32>,
    @location(8) m1: vec4<f32>,
    @location(9) m2: vec4<f32>,
    @location(10) m3: vec4<f32>,
}

struct Vout {
    @builtin(position) clip: vec4<f32>,
    @location(0) nrm: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) blend: vec3<f32>,
    @location(3) soft: vec3<f32>,
    @location(4) color: vec3<f32>,
    @location(5) eye: vec3<f32>,
    @location(6) tan: vec4<f32>,
}

// global_lit_simple's vertex wind (D_WIND_ALLOWED), in Source inches: a gust
// from the noise texture, low-frequency sway along the wind growing with the
// height above the model's pivot (mask R), and high-frequency flutter (mask
// G). params[1]: flexibility, low effects, high effects, speed multiplier.
fn wind_offset(v: Vin, world: vec3<f32>, origin: vec3<f32>) -> vec3<f32> {
    let p = source_space(world);
    let o = source_space(origin);
    let mask = textureSampleLevel(color3, samp, v.uv * u.scales.xy + u.offset[0].xy, 0.0).rg;
    let w = u.params[1];
    let at = mix(p.xy, o.xy, w.x);
    let freq = WIND_STRENGTH.w * vec4<f32>(0.125, 0.25, 1.0, 1.5);
    let noise = textureSampleLevel(normal1, samp, at * 0.00390625, 0.0).rg * 2.0 - 1.0;
    let q = vec3<f32>(at * 2.0, 0.0) + WIND_OFFSET * u.time * w.w;
    var a = freq * vec4<f32>(q, q.x + q.y + q.z);
    a = fract(fract(a) * 2.0 - 0.5) * 2.0 - 1.0;
    let waves = abs(a) * abs(a) * (3.0 - 2.0 * abs(a));
    let gust = clamp(textureSampleLevel(normal1, samp, p.xy * 0.000122 + u.time * 0.007, 0.0).r, 0.0, 1.0) * WIND_STRENGTH.x;
    let low = (waves.x + waves.y) * gust * mask.x * WIND_STRENGTH.y * w.y;
    let high = (waves.z + waves.w) * gust * mask.y * WIND_STRENGTH.z * w.z;
    let dir = vec3<f32>(WIND_DIR, 0.0);
    let height = (world.y - origin.y) / INCH;
    let flutter = normalize(vec3<f32>(noise * 2.0, 0.0) + dir) * high * max(height, 128.0);
    let d = dir * low * height + flutter;
    // Back to map space (x, y, z) = (src y, src z, src x), in metres.
    return vec3<f32>(d.y, d.z, d.x) * INCH;
}

@vertex
fn vs(v: Vin) -> Vout {
    var o: Vout;
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    var world = (model * vec4<f32>(v.pos, 1.0)).xyz;
    if (u.kind == KIND_LIT && (u.features & F_ENABLE_WIND) != 0u) {
        world += wind_offset(v, world, v.m3.xyz);
    }
    o.clip = u.view_proj * vec4<f32>(world - u.camera, 1.0);
    o.nrm = normalize((model * vec4<f32>(v.nrm, 0.0)).xyz);
    o.tan = vec4<f32>(normalize((model * vec4<f32>(v.tan.xyz, 0.0)).xyz + vec3<f32>(1.0e-6)), v.tan.w);
    o.uv = v.uv;
    o.blend = v.blend;
    o.soft = v.soft;
    o.color = v.color;
    o.eye = world - u.camera;
    return o;
}

fn has(f: u32) -> bool {
    return (u.features & f) != 0u;
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

// The fixed rational curve Dota tonemaps with, then the sRGB encode.
fn tonemap(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0)) * 0.6;
    let mapped = x * (15.8 * x + 2.12) / (x * (1.2 * x + 5.92) + 1.9);
    return linear_to_srgb(clamp(mapped, vec3<f32>(0.0), vec3<f32>(1.0)));
}

fn layer_uv(uv: vec2<f32>, scale: f32, rotation: f32, offset: vec2<f32>) -> vec2<f32> {
    let s = 1.0 / max(abs(scale), 0.0001);
    let r = radians(rotation);
    let cs = cos(r);
    let sn = sin(r);
    let p = uv - vec2<f32>(0.5, 0.5);
    let turned = vec2<f32>(cs * p.x - sn * p.y, sn * p.x + cs * p.y);
    return s * turned + vec2<f32>(0.5, 0.5) + offset;
}

fn modulate(factor: f32, mask: f32, softness: f32) -> f32 {
    let minb = max(0.0, mask - softness);
    let maxb = min(1.0, mask + softness);
    return smoothstep(minb, maxb, factor);
}

fn tint_of(channel: i32, uv: vec2<f32>) -> vec3<f32> {
    let mask = textureSample(mask_tex, samp, uv);
    var amount = mask.r;
    var base = u.tint_b[0].rgb;
    var tint = u.tints[0].rgb;
    if (channel == 1) {
        amount = mask.g;
        base = u.tint_b[1].rgb;
        tint = u.tints[1].rgb;
    } else if (channel == 2) {
        amount = mask.b;
        base = u.tint_b[2].rgb;
        tint = u.tints[2].rgb;
    } else if (channel == 3) {
        amount = mask.a;
        base = u.tint_b[3].rgb;
        tint = u.tints[3].rgb;
    }
    return srgb_to_linear(mix(base, tint, amount));
}

// A tangent space normal from an RGB normal map, glTF's layout.
fn unpack_normal(texel: vec4<f32>, strength: f32) -> vec3<f32> {
    var n = texel.xyz * 2.0 - 1.0;
    n = vec3<f32>(n.xy * strength, max(n.z, 0.0001));
    return normalize(n);
}

fn to_world(n_ts: vec3<f32>, n: vec3<f32>, t: vec4<f32>) -> vec3<f32> {
    let tangent = normalize(t.xyz - n * dot(n, t.xyz));
    let bitangent = cross(n, tangent) * select(1.0, -1.0, t.w < 0.0);
    return normalize(tangent * n_ts.x + bitangent * n_ts.y + n * n_ts.z);
}

// 3x3 filtered lookup of the sun shadow map, 1 where the sun reaches.
fn sun_shadow(world: vec3<f32>, n: vec3<f32>) -> f32 {
    let p = u.shadow_view_proj * vec4<f32>(world + n * 0.06, 1.0);
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

// deferred_global_light's cloud shadow: two scrolling layers of the cloud
// texture projected down the sun onto the ground (Source inches), the darker
// one wins; it multiplies the sun shadow.
fn cloud_shadow(world: vec3<f32>) -> f32 {
    if (u.clouds2.y < 0.5) {
        return 1.0;
    }
    let p = source_space(world);
    let to_sun = vec3<f32>(u.sun_dir.z, u.sun_dir.x, u.sun_dir.y);
    let ground = p.xy - to_sun.xy / max(to_sun.z, 0.2) * p.z;
    let uv = ground * u.clouds2.x;
    let a = textureSampleLevel(cloud_tex, samp, uv + u.clouds.xy * u.time, 0.0).r;
    let b = textureSampleLevel(cloud_tex, samp, uv + u.clouds.zw * u.time, 0.0).r;
    return srgb_to_linear(vec3<f32>(clamp(min(a, b), 0.0, 1.0))).x;
}

// CalculateDotaGlobalLight: the sun where it reaches, the directional ambient
// that the shadow darkens by its shadow amount, and the sky and ground fill
// where the sun does not reach.
fn global_light(n: vec3<f32>, shadow: f32, ao: f32) -> vec3<f32> {
    let sun_lit = clamp(dot(n, u.sun_dir.xyz), 0.0, 1.0) * shadow;
    // SSAO's terms in Dota: the sun by 0.45 + 0.55 (1 - ao)^1.85, the ambient
    // and fill by (1 - ao)^2.1.
    let open = 1.0 - ao;
    let sun = u.sun_color.rgb * sun_lit * (0.45 + 0.55 * pow(open, 1.85));
    let dark = pow(open, 2.1);
    let ambient_shadow = mix(1.0, shadow, u.ambient_dir.w);
    let directional = u.ambient_color.rgb * clamp(dot(n, u.ambient_dir.xyz), 0.0, 1.0) * ambient_shadow * dark;
    let fill = (u.sky_vertical.rgb * n.y + u.sky_constant.rgb) * (1.0 - sun_lit) * dark;
    return sun + directional + fill;
}

// The env_deferred_light_client_only lights whose radius reaches this point.
fn point_lights(world: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let col = i32(floor((world.x - u.light_grid.x) / u.light_grid.z));
    let row = i32(floor((world.z - u.light_grid.y) / u.light_grid.z));
    let cols = i32(u.light_grid.w);
    let rows = i32(u.light_grid2.x);
    if (col < 0 || row < 0 || col >= cols || row >= rows) {
        return vec3<f32>(0.0);
    }
    let at = u32(row * cols + col) * (u32(u.light_grid2.y) + 1u);
    let count = cells[at];
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < count; i++) {
        let index = cells[at + 1u + i];
        let p = lights[index * 2u];
        let color = lights[index * 2u + 1u].rgb;
        let to_light = p.xyz - world;
        let d = length(to_light);
        // deferred_local_light: smoothstep of (1 - d / r) / distance_falloff,
        // whose default in client.dll is 0.5, times N.L; no highlight.
        let t = clamp((1.0 - d / max(p.w, 0.001)) / 0.5, 0.0, 1.0);
        let facing = max(dot(n, to_light / max(d, 0.001)), 0.0);
        sum += color * (t * t * (3.0 - 2.0 * t)) * facing;
    }
    return sum;
}

// First-person distance fog in the lightinfo's fog colour, which is also the
// sky's horizon colour, so far geometry melts into the sky. Dota's own fog
// (1250..4500 inches) is tuned for its high camera; from the ground the view
// reaches much further, so it starts later and thickens over a longer range.
const FOG_START: f32 = 25.0;
const FOG_END: f32 = 140.0;

fn distance_fog(eye: vec3<f32>) -> f32 {
    let t = clamp((length(eye) - FOG_START) / (FOG_END - FOG_START), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// deferred_post_process's height fog (D_HEIGHT_FOG, type 0), in Source inches,
// with the four default lightinfos' keys: falloff 0.125 over a fog space of
// 10 / max_z (512) per inch, two layers of fow_clouds_01 at scale 1024 (layer 0
// turned -33 degrees and scrolled 33, -33 per second, layer 1 flowing along the
// scroll with two offset phases), r_dota_height_fog_plane_height 128, faded
// in from the 1024 inch exclusion radius to r_dota_height_fog_exclusion_radius
// 2200. The opacity map is 1 over the whole map and is left out. Dota samples
// the noise midway to where the ray meets that plane; from the ground that
// point jumps at eye level and cuts the fog in two, so the fragment is used.
fn height_fog(world: vec3<f32>) -> f32 {
    if (u.height_fog.w <= 0.0 || u.clouds2.y < 0.5) {
        return 0.0;
    }
    let p = source_space(world);
    let cam = source_space(u.camera);
    let v = p - cam;
    let dist = length(v);
    let ex = clamp((dist - 1024.0) / (2200.0 - 1024.0), 0.0, 1.0);
    let m = p.xy;
    let r0 = -0.5759587;
    let a0 = cos(r0) / 1024.0;
    let b0 = sin(r0) / 1024.0;
    let s0 = u.time * vec2<f32>(33.0, -33.0);
    let uv0 = vec2<f32>(
        a0 * m.x + b0 * m.y + 0.5 - 0.5 * (a0 - b0) - (a0 * s0.x + b0 * s0.y),
        b0 * m.x - a0 * m.y + 0.5 - 0.5 * (a0 + b0) - (b0 * s0.x - a0 * s0.y),
    );
    let n0 = textureSampleLevel(cloud_tex, samp, uv0, 0.0).g;
    let uv1 = vec2<f32>(m.x, -m.y) / 1024.0 + (0.5 - 0.5 / 1024.0);
    let dir = vec2<f32>(-0.70710677, -0.70710677);
    let ph = u.time / (2.0 * 1024.0 / 46.669048);
    let pa = ph + 0.5;
    let fa = fract(pa);
    let fb = fract(ph);
    let sa = textureSampleLevel(cloud_tex, samp, uv1 + (floor(pa) * 0.311 + 0.5) + (fa - 0.5) * dir, 0.0).g;
    let sb = textureSampleLevel(cloud_tex, samp, uv1 + floor(ph) * 0.311 + (fb - 0.5) * dir, 0.0).g;
    let n1 = sa * abs(2.0 * fb - 1.0) + sb * abs(2.0 * fa - 1.0);
    let k = 0.125;
    let d = v * (10.0 / 512.0);
    let kd = k * d.z;
    var column = length(d) * exp(-k * cam.z * (10.0 / 512.0));
    if (abs(kd) > 0.0001) {
        column *= (1.0 - exp(-kd)) / kd;
    }
    return clamp(u.height_fog.w * ex * ex * n0 * n1 * column, 0.0, 1.0);
}

fn illumination(world: vec3<f32>, n: vec3<f32>, shadow: f32) -> vec3<f32> {
    return global_light(n, shadow, occlusion(world, n)) + point_lights(world, n);
}

// The Phong highlight of the lightinfo's specular light (CalculateDotaSpecular).
fn dota_specular(world: vec3<f32>, n: vec3<f32>) -> f32 {
    let to_camera = normalize(u.camera - world);
    let facing = clamp(dot(u.specular_dir.xyz, reflect(to_camera, n)), 0.0, 1.0);
    return pow(facing, u.specular_dir.w);
}

fn source_space(p: vec3<f32>) -> vec3<f32> {
    let q = (p - MAP_OFFSET) / INCH;
    return vec3<f32>(q.z, q.x, q.y);
}

// water_dota: a scrolling normal map, fresnel and a Blinn highlight towards the eye.
// water_dota's low-end pixel shader (combo 215), in Source space (inches, Z up):
// two normal-map samples scrolled along the map's flow map with offset phases,
// a fresnel surface colour, a highlight of LowEndReflectionDir, lit by the global
// light. params: [0] surface colour, factor; [1] reflection colour, power;
// [2] reflection dir, minimum colour; [3] opacity, base bloom, bump, flow time
// scale. tints: [0] world position scale, noise uv scale, noise strength, lerp
// exponent; [1] 1/normal uv scale, flow interval (s), flow scroll distance.
// colour1 is g_tNoise, colour2 the map's water_flow_map.
fn water(i: Vout, world: vec3<f32>, shadow: f32) -> vec4<f32> {
    let p = source_space(world);
    let eye = source_space(u.camera);
    let flow = textureSample(color2, samp, (p.xy + 9472.0) / 18944.0).xy * 2.0 - 1.0;
    let scroll = flow * u.tints[1].z;
    let bump = (dot(flow, flow) + 0.1) * u.params[3].z;
    let base = vec2<f32>(p.x, -p.y) * u.tints[0].x;
    let t = u.time * u.params[3].w / (2.0 * u.tints[1].y)
        + textureSample(color1, samp, base * u.tints[0].y).g * u.tints[0].z;
    let ta = t + 0.5;
    let fa = fract(ta);
    let fb = fract(t);
    let uva = base * u.tints[1].x + (floor(ta) * 0.311 + 0.5) + fa * scroll;
    let uvb = base * u.tints[1].x + floor(t) * 0.311 + fb * scroll;
    let wa = pow(abs(2.0 * fa - 1.0), u.tints[0].w);
    let wb = pow(abs(2.0 * fb - 1.0), u.tints[0].w);
    let nab = (textureSample(normal0, samp, uva).rg * wb + textureSample(normal0, samp, uvb).rg * wa) * 2.0 - 1.0;
    var nxy = nab * bump;
    if (dot(nxy, nxy) > 1.0) {
        nxy = normalize(nxy);
    }
    let n = vec3<f32>(nxy, sqrt(max(1.0 - dot(nxy, nxy), 0.0)));
    let v = normalize(p - eye);
    let r = 2.0 * dot(v, n) * n - v;
    let fres = clamp(pow(max(1.0 - n.z, 0.0), u.params[0].w) + u.params[2].w, 0.0, 1.0);
    let s = clamp(dot(u.params[2].xyz, r), 0.0, 1.0);
    let sp = pow(s, u.params[1].w);
    let surf = clamp(u.params[0].rgb * fres + u.params[1].rgb * sp, vec3<f32>(0.0), vec3<f32>(1.0));
    let light = global_light(vec3<f32>(n.y, n.z, n.x), shadow, 0.0);
    let rgb = mix(surf * light, surf, clamp(2.0 * s + u.params[3].y, 0.0, 1.0));
    return vec4<f32>(rgb, clamp(sp + u.params[3].x, 0.0, 1.0));
}

@fragment
fn fs(i: Vout, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let world = i.eye + u.camera;
    var vn = normalize(i.nrm);
    if (!front) {
        vn = -vn;
    }
    if (u.kind == KIND_WATER) {
        let w = water(i, world, sun_shadow(world, vn) * cloud_shadow(world));
        let haze = distance_fog(i.eye);
        let low = mix(w.rgb, u.height_fog.rgb, height_fog(world));
        return vec4<f32>(tonemap(mix(low, u.fog_color.rgb, haze) * u.sun_dir.w), w.a);
    }
    var albedo: vec3<f32>;
    var alpha = 1.0;
    var n = vn;
    var spec = vec4<f32>(0.0);
    if (u.kind == KIND_MULTIBLEND) {
        let uv0 = layer_uv(i.uv, u.scales.x, u.rotate.x, u.offset[0].xy);
        let uv1 = layer_uv(i.uv, u.scales.y, u.rotate.y, u.offset[1].xy);
        let uv2 = layer_uv(i.uv, u.scales.z, u.rotate.z, u.offset[2].xy);
        let uv3 = layer_uv(i.uv, u.scales.w, u.rotate.w, u.offset[3].xy);
        let c0 = textureSample(color0, samp, uv0);
        let c1 = textureSample(color1, samp, uv1);
        let c2 = textureSample(color2, samp, uv2);
        let c3 = textureSample(color3, samp, uv3);
        let w1 = modulate(i.blend.x, c1.a, i.soft.x);
        let inv1 = 1.0 - w1;
        let w2 = min(modulate(i.blend.y, c2.a, i.soft.y), inv1);
        let inv2 = inv1 - w2;
        let w3 = min(modulate(i.blend.z, c3.a, i.soft.z), inv2);
        let w0 = inv2 - w3;
        albedo = (srgb_to_linear(c0.rgb) * tint_of(0, uv0) * w0
            + srgb_to_linear(c1.rgb) * tint_of(1, uv1) * w1
            + srgb_to_linear(c2.rgb) * tint_of(2, uv2) * w2
            + srgb_to_linear(c3.rgb) * tint_of(3, uv3) * w3) * i.color;
        let nt = textureSample(normal0, samp, uv0) * w0 + textureSample(normal1, samp, uv1) * w1
            + textureSample(normal2, samp, uv2) * w2 + textureSample(normal3, samp, uv3) * w3;
        n = to_world(unpack_normal(nt, u.params[0].y), vn, i.tan);
        spec = textureSample(spec0, samp, uv0) * w0 + textureSample(spec1, samp, uv1) * w1
            + textureSample(spec2, samp, uv2) * w2 + textureSample(spec3, samp, uv3) * w3;
        spec.x *= u.params[0].x;
    } else {
        // F_SCROLL_UV: params[2].xy direction * speed, z the mode; colour1 is g_tScrollSpeed.
        var uv = i.uv * u.scales.xy + u.offset[0].xy;
        var uv_color = uv;
        var scroll = vec2<f32>(1.0);
        if (u.params[2].z > 0.5) {
            scroll = textureSample(color1, samp, uv).ga;
            uv_color = uv + u.params[2].xy * u.time * scroll.x;
            if (u.params[2].z > 1.5) {
                uv = uv_color;
            }
        }
        let texel = textureSample(color0, samp, uv_color);
        let normal_texel = textureSample(normal0, samp, uv);
        let spec_texel = textureSample(spec0, samp, uv);
        let mask = textureSample(mask_tex, samp, uv);
        var tint = srgb_to_linear(u.tints[0].rgb);
        if (has(F_TINT_MASK)) {
            tint = mix(vec3<f32>(1.0), tint, mask.g);
        }
        if (u.kind == KIND_SOLID) {
            albedo = tint;
        } else {
            albedo = srgb_to_linear(texel.rgb) * tint * i.color;
            alpha = texel.a;
            if (has(F_TRANSLUCENT) && u.params[2].z > 0.5) {
                alpha = scroll.y;
            }
            alpha *= u.mask_scale;
        }
        if (u.params[2].w > 0.0) {
            // F_ALPHA_ANGLE_POWER: cards fade as they turn edge-on to the eye.
            alpha *= pow(abs(dot(normalize(u.camera - world), vn)), u.params[2].w);
        }
        if (has(F_ALPHA_TEST) && alpha < u.params[0].z) {
            discard;
        }
        if (!has(F_TRANSLUCENT)) {
            alpha = 1.0;
        }
        if (has(F_NORMAL_MAP)) {
            n = to_world(unpack_normal(normal_texel, u.params[0].y), vn, i.tan);
        }
        if (has(F_HERO)) {
            // hero.vfx: the specular intensity is masks 2's red, the self
            // illumination masks 1's alpha.
            let masks1 = textureSample(spec1, samp, uv);
            spec = vec4<f32>(spec_texel.r * u.params[0].x, masks1.a, 0.0, 0.0);
        } else if (has(F_SPECULAR)) {
            spec = spec_texel;
            spec.x *= u.params[0].x;
            if (has(F_MODULATE_SPECULAR_BY_ALPHA)) {
                spec.x *= alpha;
            }
        }
    }
    // Translucent materials are unlit unless they ask for light; overlays are
    // decals on the lit surface.
    let unlit = has(F_FULLBRIGHT)
        || (has(F_TRANSLUCENT) && !has(F_ALLOW_LIGHTING_ON_TRANSLUCENT) && !has(F_OVERLAY));
    var lit: vec3<f32>;
    if (unlit) {
        lit = albedo;
    } else {
        let shadow = sun_shadow(world, vn) * cloud_shadow(world);
        let light = illumination(world, n, shadow);
        let specular = dota_specular(world, n) * spec.x;
        let specular_albedo = albedo + specular * u.specular_albedo.rgb;
        lit = light * specular_albedo + specular * u.specular_lit.rgb * shadow;
        // Self illumination shows the unlit albedo.
        lit = mix(lit, specular_albedo, spec.y);
    }
    if (has(F_CRYSTAL)) {
        // crystal.vfx: the inside glows with its own colour and the rim
        // reflects the sky, more opaque where it is seen edge on.
        let view = normalize(u.camera - world);
        let rim = pow(1.0 - clamp(dot(n, view), 0.0, 1.0), max(u.params[3].x, 0.5));
        lit = lit + albedo * 0.7 + u.fog_color.rgb * rim * 0.9;
        alpha = clamp(mix(alpha, 1.0, rim), 0.0, 1.0);
    }
    if (u.kind == KIND_SOLID) {
        // The black backdrop past the map edge sits under Dota's fog from its
        // camera; seen from the ground it gets that fog (fog_start 1250,
        // fog_end 4500, exponent 2 of ent_dota_lightinfo) instead of a black wall.
        // Near it keeps a darker shade of the fog, so it reads as a silhouette.
        let t = clamp((length(i.eye) - 1250.0 * INCH) / ((4500.0 - 1250.0) * INCH), 0.0, 1.0);
        lit = u.fog_color.rgb * mix(0.45, 1.0, t * t);
    }
    let low = height_fog(world);
    lit = mix(lit, u.height_fog.rgb, low);
    let haze = distance_fog(i.eye);
    lit = mix(lit, u.fog_color.rgb, haze);
    if (has(F_ADDITIVE)) {
        // Dota adds these to the linear HDR colour before its tonemap. The
        // target here is already encoded, so add the tonemapped linear value:
        // sRGB-encoding a small addition would brighten it many times over.
        let x = max(lit * u.sun_dir.w, vec3<f32>(0.0)) * 0.6;
        let mapped = clamp(x * (15.8 * x + 2.12) / (x * (1.2 * x + 5.92) + 1.9), vec3<f32>(0.0), vec3<f32>(1.0));
        return vec4<f32>(mapped * alpha * (1.0 - haze) * (1.0 - low), alpha);
    }
    let color = tonemap(lit * u.sun_dir.w);
    return vec4<f32>(color, alpha);
}

// Depth of the sun shadow map: alpha tested like the colour pass.
@fragment
fn fs_shadow(i: Vout) {
    if (u.kind == KIND_LIT && has(F_ALPHA_TEST)) {
        let texel = textureSample(color0, samp, i.uv * u.scales.xy + u.offset[0].xy);
        if (texel.a * u.mask_scale < u.params[0].z) {
            discard;
        }
    }
}

// grasstile (vertex combo 0, forward pixel combo 9): a camera-independent card
// per blade, leaned by the skew map, swayed by the wind and faded out between
// 1000 and 2500 inches. Instance: m0 root, width; m1 right, height; m2 skew,
// brightness; m3 tile tint, material + 4 * mirrored. params[material]: flex,
// atlas u0, u1, specular intensity.
const GRASS_WIND_STRENGTH: f32 = 0.25;
const GRASS_WIND_FREQUENCY: f32 = 0.03;
const GRASS_WIND_OFFSET: vec3<f32> = vec3<f32>(0.25, 0.2, 0.21);
const GRASS_WIND_DIR: vec2<f32> = vec2<f32>(0.707, 0.707);

@vertex
fn vs_grass(v: Vin) -> Vout {
    var o: Vout;
    let side = v.pos.x;
    let top = v.pos.y;
    let root = v.m0.xyz;
    let width = v.m0.w;
    let right = v.m1.xyz;
    let height = v.m1.w;
    let skew = v.m2.xyz;
    let material = u32(v.m3.w) & 3u;
    let mirrored = v.m3.w >= 4.0;
    let m = u.params[material];
    let lean = length(skew);
    let sdir = skew / max(lean, 1.0e-6);
    let src = source_space(root);
    let q = vec3<f32>(src.x, src.y, 0.0) * GRASS_WIND_FREQUENCY + GRASS_WIND_OFFSET * u.time;
    var a = vec4<f32>(q, q.x + q.y + q.z) * vec4<f32>(0.5, 1.0, 0.5, 1.5);
    a = fract(fract(a) * 2.0 - 0.5) * 2.0 - 1.0;
    let t = abs(a);
    let s = t * t * (3.0 - 2.0 * t);
    let wind = dot(s, vec4<f32>(1.0, 0.5, 0.25, 0.05)) * GRASS_WIND_STRENGTH
        * vec3<f32>(GRASS_WIND_DIR.y, 0.0, GRASS_WIND_DIR.x);
    let wf = wind * m.x;
    let base = root + right * (side * 2.0 - 1.0) * 0.5 * width;
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let world = base + up * height * (1.0 - lean) * top + (lean * sdir + (1.0 - lean) * wf) * height * top;
    o.clip = u.view_proj * vec4<f32>(world - u.camera, 1.0);
    o.eye = world - u.camera;
    o.nrm = normalize(up - wf * (height / INCH) * 0.01);
    o.tan = vec4<f32>(right, 1.0);
    let uvx = select(side, 1.0 - side, mirrored);
    o.uv = vec2<f32>(mix(m.y, m.z, uvx), 1.0 - top);
    o.color = v.m3.rgb;
    let dist = length(o.eye) / INCH;
    o.soft = vec3<f32>(v.m2.w, 1.0 - clamp((dist - 1000.0) / 1500.0, 0.0, 1.0), m.w);
    o.blend = vec3<f32>(0.0);
    return o;
}

@fragment
fn fs_grass(i: Vout) -> @location(0) vec4<f32> {
    let texel = textureSample(color0, samp, i.uv);
    let spec_texel = textureSample(spec0, samp, i.uv);
    let alpha = texel.a * i.soft.y;
    if (alpha < 0.3) {
        discard;
    }
    let world = i.eye + u.camera;
    let n = normalize(i.nrm);
    let bright = i.soft.x;
    let b = clamp(bright, 0.0, 0.7) / 0.7;
    let sky_top = u.sky_vertical.rgb + u.sky_constant.rgb;
    let albedo = srgb_to_linear(texel.rgb) * i.color * mix(sky_top, vec3<f32>(1.0), b) * bright;
    let shadow = sun_shadow(world, n) * cloud_shadow(world);
    let specular = dota_specular(world, n) * spec_texel.g * i.soft.z;
    let sun_lit = clamp(dot(n, u.sun_dir.xyz), 0.0, 1.0) * shadow;
    var lit = (albedo + specular * u.specular_albedo.rgb) * illumination(world, n, shadow)
        + specular * u.specular_lit.rgb * sun_lit;
    lit = mix(lit, u.height_fog.rgb, height_fog(world));
    lit = mix(lit, u.fog_color.rgb, distance_fog(i.eye));
    return vec4<f32>(tonemap(lit * u.sun_dir.w), 1.0);
}
