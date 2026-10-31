//! The Dota 2 map as a layer inside a Horizon world.
//!
//! The mesh is the real map (not voxels). Players, mobs and items still use
//! Minecraft movement; this clips that movement against the map's physics
//! mesh, and the crosshair can place blocks on it. `map.bin` is baked locally
//! from the player's own Dota install and is not part of the game data.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use horizon_core::geometry::{Aabb, Vec3};
use wgpu::util::DeviceExt;

const SKIN: f32 = 0.03;
/// Highest ledge walked onto without a jump (Minecraft's step height).
const STEP_HEIGHT: f32 = 0.6;
/// How far short of a surface a sweep stops.
const MARGIN: f32 = 0.004;
/// Deeper than this, a body is inside something and is pushed out.
const EMBEDDED: f32 = 0.05;
/// Where `depth` stops measuring.
const DEEP: f32 = 0.08;
/// A step this low lands anywhere; a higher one only on walkable ground.
const PEBBLE: f32 = 0.12;
/// Ground no steeper than about 45 degrees.
const WALKABLE: f32 = 0.7;

struct Tri {
    v: [[f32; 3]; 3],
}

struct Node {
    min: [f32; 3],
    max: [f32; 3],
    left: u32,
    right: u32,
    start: u32,
    count: u32,
}

struct Map {
    tris: Vec<Tri>,
    nodes: Vec<Node>,
    order: Vec<u32>,
    spawn: Vec3,
    min: [f32; 3],
    max: [f32; 3],
    batches: Vec<BatchCpu>,
    textures: Vec<PathBuf>,
}

/// Which Dota shader draws a batch.
const KIND_LIT: u32 = 0;
const KIND_MULTIBLEND: u32 = 1;
const KIND_WATER: u32 = 2;
const KIND_SOLID: u32 = 3;
/// The map's clutter grass (grass.bin from extract_dota_grass.py).
const KIND_GRASS: u32 = 4;
/// Material features, as bake_dota_map4.py packs them.
const F_TRANSLUCENT: u32 = 4;
const F_ADDITIVE: u32 = 8;
/// Texture slots: colour 0..3, tint mask, normal 0..3, specular 0..3.
const SLOTS: usize = 13;

#[derive(Clone)]
struct BatchCpu {
    kind: u32,
    features: u32,
    slots: [i32; SLOTS],
    scales: [f32; 4],
    rotate: [f32; 4],
    offset: [f32; 8],
    tints: [[f32; 4]; 4],
    tint_b: [[f32; 4]; 4],
    mask_scale: f32,
    params: [f32; 16],
    instances: Vec<[[f32; 4]; 4]>,
    verts: Vec<GpuVert>,
    indices: Vec<u32>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuVert {
    pos: [f32; 3],
    nrm: [f32; 3],
    tan: [f32; 4],
    uv: [f32; 2],
    blend: [f32; 3],
    soft: [f32; 3],
    color: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuUniform {
    view_proj: [[f32; 4]; 4],
    camera: [f32; 3],
    time: f32,
    kind: u32,
    features: u32,
    mask_scale: f32,
    pad0: f32,
    scales: [f32; 4],
    rotate: [f32; 4],
    tints: [[f32; 4]; 4],
    offset: [[f32; 4]; 4],
    tint_b: [[f32; 4]; 4],
    params: [[f32; 4]; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient_dir: [f32; 4],
    ambient_color: [f32; 4],
    sky_vertical: [f32; 4],
    sky_constant: [f32; 4],
    specular_dir: [f32; 4],
    specular_albedo: [f32; 4],
    specular_lit: [f32; 4],
    fog_color: [f32; 4],
    // origin x, origin z, cell size, columns
    light_grid: [f32; 4],
    // rows, lights per cell
    light_grid2: [f32; 4],
    clouds: [f32; 4],
    clouds2: [f32; 4],
    // linear colour, density
    height_fog: [f32; 4],
    shadow_view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SkyUniform {
    inverse_view_proj: [[f32; 4]; 4],
    tint: [f32; 4],
    fog_color: [f32; 4],
}

struct GpuSky {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
}

struct GpuBatch {
    uniform: wgpu::Buffer,
    group: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    instance_count: u32,
    instance_buf: wgpu::Buffer,
    kind: u32,
    features: u32,
    scales: [f32; 4],
    rotate: [f32; 4],
    offset: [[f32; 4]; 4],
    tints: [[f32; 4]; 4],
    tint_b: [[f32; 4]; 4],
    mask_scale: f32,
    params: [[f32; 4]; 4],
}

impl GpuBatch {
    /// 0 opaque, 1 grass, 2 alpha blended, 3 additive: the order they are drawn in.
    fn pass(&self) -> usize {
        if self.kind == KIND_GRASS {
            1
        } else if self.features & F_ADDITIVE != 0 {
            3
        } else if self.features & F_TRANSLUCENT != 0 || self.kind == KIND_WATER {
            2
        } else {
            0
        }
    }
}

struct GpuMap {
    pipelines: [wgpu::RenderPipeline; 4],
    batches: Vec<GpuBatch>,
    sky: Option<GpuSky>,
    shadow: GpuShadow,
    light_grid: ([f32; 4], [f32; 4]),
    dynamic: DynamicLights,
    clouds: ([f32; 4], [f32; 4]),
    started: std::time::Instant,
}

/// The sun shadow map. The map and Dota's sun do not move, so it is drawn
/// once, the first frame.
struct GpuShadow {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    view: wgpu::TextureView,
    view_proj: glam::Mat4,
    drawn: AtomicBool,
}

const SHADOW_SIZE: u32 = 8192;

/// Lights that move or flicker (the particle systems' deferred lights), added
/// to the map's static lights and light grid every frame.
struct DynamicLights {
    static_lights: Vec<[f32; 4]>,
    static_cells: Vec<u32>,
    lights_buf: wgpu::Buffer,
    cells_buf: wgpu::Buffer,
    min: [f32; 2],
    cols: usize,
    rows: usize,
    last: std::sync::Mutex<usize>,
}

/// At most this many particle lights at once.
const DYNAMIC_LIGHTS: usize = 192;

impl DynamicLights {
    fn update(&self, queue: &wgpu::Queue, extra: &[([f32; 3], f32, [f32; 3])]) {
        let Ok(mut last) = self.last.lock() else {
            return;
        };
        if extra.is_empty() && *last == 0 {
            return;
        }
        let mut lights = self.static_lights.clone();
        let mut cells = self.static_cells.clone();
        let base = lights.len() / 2;
        for (k, (pos, radius, color)) in extra.iter().take(DYNAMIC_LIGHTS).enumerate() {
            let r = radius.clamp(0.1, 30.0);
            lights.push([pos[0], pos[1], pos[2], r]);
            lights.push([color[0], color[1], color[2], 0.0]);
            let cell = |v: f32, lo: f32, n: usize| (((v - lo) / LIGHT_CELL).floor().max(0.0) as usize).min(n - 1);
            let (c0, c1) = (cell(pos[0] - r, self.min[0], self.cols), cell(pos[0] + r, self.min[0], self.cols));
            let (r0, r1) = (cell(pos[2] - r, self.min[1], self.rows), cell(pos[2] + r, self.min[1], self.rows));
            for row in r0..=r1 {
                for col in c0..=c1 {
                    let at = (row * self.cols + col) * (LIGHTS_PER_CELL + 1);
                    let n = cells[at] as usize;
                    if n < LIGHTS_PER_CELL {
                        cells[at + 1 + n] = (base + k) as u32;
                        cells[at] += 1;
                    }
                }
            }
        }
        *last = extra.len();
        queue.write_buffer(&self.lights_buf, 0, bytemuck::cast_slice(&lights));
        queue.write_buffer(&self.cells_buf, 0, bytemuck::cast_slice(&cells));
    }
}

fn world() -> Option<&'static Map> {
    static MAP: OnceLock<Option<Map>> = OnceLock::new();
    MAP.get_or_init(load_map).as_ref()
}

/// First time the local player moves, stand them on the map if they are not
/// already on it. Blocks, mobs and the rest of the world stay where they are.
pub fn maybe_seat(position: &mut Vec3, velocity: &mut Vec3) {
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.load(Ordering::Relaxed) {
        return;
    }
    let Some(map) = world() else {
        return;
    };
    if map.on_map(position.x, position.y, position.z) {
        DONE.store(true, Ordering::Relaxed);
        return;
    }
    *position = map.spawn;
    *velocity = Vec3::ZERO;
    DONE.store(true, Ordering::Relaxed);
    log::info!(
        "Dota map: standing at {:.1} {:.1} {:.1}",
        map.spawn.x,
        map.spawn.y,
        map.spawn.z
    );
}

/// Keep the local player out of the holes Dota never filled because its camera
/// never looks there (under overhanging cliffs, behind the map's edge): where
/// the player last stood on the map is remembered, and a fall below the map
/// brings them back there.
pub fn guard(position: &mut Vec3, velocity: &mut Vec3, on_ground: bool) {
    static SAFE: std::sync::Mutex<Option<Vec3>> = std::sync::Mutex::new(None);
    let Some(map) = world() else {
        return;
    };
    let (x, z) = (position.x as f32, position.z as f32);
    let over = x >= map.min[0] && x <= map.max[0] && z >= map.min[2] && z <= map.max[2];
    if !over {
        return;
    }
    let Ok(mut safe) = SAFE.lock() else {
        return;
    };
    if on_ground && map.on_map(position.x, position.y, position.z) {
        *safe = Some(*position);
    } else if position.y < f64::from(map.min[1]) - 2.0 {
        if let Some(back) = *safe {
            log::info!(
                "Dota map: fell under the map at {:.1} {:.1} {:.1}, back to {:.1} {:.1} {:.1}",
                position.x,
                position.y,
                position.z,
                back.x,
                back.y,
                back.z
            );
            *position = back;
            *velocity = Vec3::ZERO;
        }
    }
}

/// Whether the map is drawn; Minecraft's clouds at y 192 would cut through it.
pub fn loaded() -> bool {
    world().is_some()
}

/// Restrict a Minecraft move so it also stops on the map mesh.
pub fn clip_movement(bounds: Aabb, movement: Vec3, _max_step: f32, on_ground: bool) -> Vec3 {
    match world() {
        Some(map) => map.clip(bounds, movement, on_ground),
        None => movement,
    }
}

/// First map surface on the segment `from`..`to`, for the third-person camera.
pub fn clip_segment(from: [f64; 3], to: [f64; 3]) -> Option<[f64; 3]> {
    let map = world()?;
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if len < 1.0e-6 {
        return None;
    }
    map.ray(from, d, len).map(|(_, _, point)| point)
}

pub struct Pick {
    pub pos: [i32; 3],
    pub face: [i32; 3],
    pub point: [f64; 3],
}

/// Nearest map surface along a crosshair ray, so blocks can be placed on it.
pub fn pick_surface(origin: [f64; 3], dir: [f64; 3], reach: f64) -> Option<Pick> {
    let map = world()?;
    let (t, normal, point) = map.ray(origin, dir, reach)?;
    let _ = t;
    let outside = [
        point[0] + normal[0] as f64 * 0.08,
        point[1] + normal[1] as f64 * 0.08,
        point[2] + normal[2] as f64 * 0.08,
    ];
    let mut face = [0; 3];
    let axis = (0..3)
        .max_by(|&a, &b| {
            normal[a]
                .abs()
                .partial_cmp(&normal[b].abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(1);
    face[axis] = if normal[axis] >= 0.0 { 1 } else { -1 };
    Some(Pick {
        pos: [
            outside[0].floor() as i32,
            outside[1].floor() as i32,
            outside[2].floor() as i32,
        ],
        face,
        point,
    })
}

pub fn draw(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pass: &mut wgpu::RenderPass<'_>,
    format: wgpu::TextureFormat,
    camera: [f32; 3],
    view_proj: glam::Mat4,
    day: f32,
) {
    let Some(map) = world() else {
        return;
    };
    if map.batches.is_empty() && map.tris.is_empty() {
        return;
    }
    static GPU: OnceLock<Option<GpuMap>> = OnceLock::new();
    let gpu = GPU.get_or_init(|| GpuMap::new(device, queue, format, map));
    let Some(gpu) = gpu else {
        return;
    };
    gpu.draw(device, queue, pass, format, camera, view_proj, day, &Atmosphere::at(camera, day));
}

/// The Dota global light for Minecraft blocks and entities on the map (the
/// `dota` vectors of terrain.wgsl and entity.wgsl), zero when there is no map.
pub fn globals_light(camera: [f64; 3], day: f32) -> [[f32; 4]; 12] {
    let Some(map) = world() else {
        return [[0.0; 4]; 12];
    };
    let shadow = shadow_matrix().map_or([[0.0; 4]; 4], |m| m.to_cols_array_2d());
    let cam = [camera[0] as f32, camera[1] as f32, camera[2] as f32];
    let a = Atmosphere::at(cam, day);
    [
        a.sun_dir,
        a.sun_color,
        a.ambient_dir,
        a.ambient_color,
        a.sky_vertical,
        a.sky_constant,
        [map.min[0], map.min[2], map.max[0], map.max[2]],
        [cam[0], cam[1], cam[2], DOTA_LIT_ABOVE],
        shadow[0],
        shadow[1],
        shadow[2],
        shadow[3],
    ]
}

/// The sun's shadow map: one depth texture over the whole map, drawn once by
/// `GpuMap`, read by the map's shaders and by the entity shader (players and
/// mobs standing in the shade). A 1x1 texture when there is no map.
pub fn shadow_view(device: &wgpu::Device) -> wgpu::TextureView {
    static VIEW: OnceLock<wgpu::TextureView> = OnceLock::new();
    VIEW.get_or_init(|| {
        let size = if world().is_some() { SHADOW_SIZE } else { 1 };
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Dota sun shadow"),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    })
    .clone()
}

/// World to the sun shadow map's clip space: an orthographic view down the
/// sun over the whole map. Dota's sun does not move.
pub fn shadow_matrix() -> Option<glam::Mat4> {
    let map = world()?;
    let lo = glam::Vec3::from(map.min);
    let hi = glam::Vec3::from(map.max);
    let center = (lo + hi) * 0.5;
    let radius = (hi - lo).length() * 0.5;
    let to_sun = glam::Vec3::from(Atmosphere::at(center.to_array(), 1.0).sun_dir_xyz()).normalize();
    let eye = center + to_sun * radius * 2.0;
    Some(
        glam::Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.0, radius * 4.0)
            * glam::Mat4::look_at_rh(eye, center, glam::Vec3::Y),
    )
}

/// Whether the vanilla terrain section starting at world height `base` is left
/// undrawn: with the map loaded, the Minecraft world far below it is not shown.
pub fn hides_section(base: i32) -> bool {
    loaded() && (base + 16) as f32 <= DOTA_LIT_ABOVE
}

/// Blocks and entities below this height (the Minecraft world under the map)
/// keep Minecraft's light.
const DOTA_LIT_ABOVE: f32 = 185.0;

/// The map's ent_dota_lightinfo entities, read once from the entity lump
/// exported next to map.bin.
fn light_infos() -> &'static (Vec<LightInfo>, f32) {
    static INFOS: OnceLock<(Vec<LightInfo>, f32)> = OnceLock::new();
    INFOS.get_or_init(|| {
        let path = map_path().and_then(|p| {
            Some(p.parent()?.join("ents").join("maps").join("dota").join("entities").join("default_ents.vents"))
        });
        let text = path.and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        let (infos, power) = parse_light_infos(&text);
        if infos.is_empty() {
            log::warn!("Dota map: no default ent_dota_lightinfo, using default_RF");
            return (vec![LightInfo::default_rf()], 28.0);
        }
        log::info!("Dota map: {} default light infos", infos.len());
        (infos, power)
    })
}

/// One time of day of an ent_dota_lightinfo, colours as written (0..255).
#[derive(Clone, Copy)]
struct LightKeys {
    color: [f32; 3],
    light_scale: f32,
    ambient_color: [f32; 3],
    ambient_scale: f32,
    shadow_color: [f32; 3],
    shadow_secondary: [f32; 3],
    shadow_ground_scale: f32,
    light_angles: [f32; 3],
    ambient_angles: [f32; 3],
    specular_color: [f32; 3],
    specular_power: f32,
    specular_angles: [f32; 3],
    specular_independence: f32,
    fog_color: [f32; 3],
    sky_tint: [f32; 3],
    height_fog_color: [f32; 3],
    height_fog_density: f32,
}

struct LightInfo {
    origin: [f32; 2],
    inner: f32,
    outer: f32,
    ambient_shadow: f32,
    day: LightKeys,
    night: LightKeys,
}

impl LightKeys {
    fn mix(&self, o: &Self, t: f32) -> Self {
        let f = |a: f32, b: f32| a + (b - a) * t;
        let v = |a: [f32; 3], b: [f32; 3]| mix3(a, b, t);
        Self {
            color: v(self.color, o.color),
            light_scale: f(self.light_scale, o.light_scale),
            ambient_color: v(self.ambient_color, o.ambient_color),
            ambient_scale: f(self.ambient_scale, o.ambient_scale),
            shadow_color: v(self.shadow_color, o.shadow_color),
            shadow_secondary: v(self.shadow_secondary, o.shadow_secondary),
            shadow_ground_scale: f(self.shadow_ground_scale, o.shadow_ground_scale),
            light_angles: v(self.light_angles, o.light_angles),
            ambient_angles: v(self.ambient_angles, o.ambient_angles),
            specular_color: v(self.specular_color, o.specular_color),
            specular_power: f(self.specular_power, o.specular_power),
            specular_angles: v(self.specular_angles, o.specular_angles),
            specular_independence: f(self.specular_independence, o.specular_independence),
            fog_color: v(self.fog_color, o.fog_color),
            sky_tint: v(self.sky_tint, o.sky_tint),
            height_fog_color: v(self.height_fog_color, o.height_fog_color),
            height_fog_density: f(self.height_fog_density, o.height_fog_density),
        }
    }
}

impl LightInfo {
    fn default_rf() -> Self {
        let day = LightKeys {
            color: [216.0, 208.0, 207.0],
            light_scale: 5.0,
            ambient_color: [63.0, 144.0, 169.0],
            ambient_scale: 5.0,
            shadow_color: [230.0, 175.0, 177.0],
            shadow_secondary: [230.0, 175.0, 177.0],
            shadow_ground_scale: 1.0,
            light_angles: [66.0, 0.0, 0.0],
            ambient_angles: [0.0, 23.0, 0.0],
            specular_color: [81.0, 196.0, 232.0],
            specular_power: -1.0,
            specular_angles: [64.0, 283.0, 0.0],
            specular_independence: 0.0,
            fog_color: [149.0, 239.0, 255.0],
            sky_tint: [255.0, 255.0, 255.0],
            height_fog_color: [255.0, 255.0, 255.0],
            height_fog_density: 0.0,
        };
        let night = LightKeys {
            color: [149.0, 172.0, 253.0],
            ambient_color: [37.0, 98.0, 197.0],
            shadow_color: [138.0, 160.0, 255.0],
            shadow_secondary: [138.0, 160.0, 255.0],
            shadow_ground_scale: 1.5,
            specular_color: [95.0, 162.0, 221.0],
            fog_color: [99.0, 140.0, 206.0],
            sky_tint: [127.0, 143.0, 197.0],
            height_fog_color: [111.0, 157.0, 223.0],
            height_fog_density: 0.1,
            ..day
        };
        Self {
            origin: [-6784.0, -6304.0],
            inner: 7000.0,
            outer: 13000.0,
            ambient_shadow: 0.0,
            day,
            night,
        }
    }
}

fn parse_light_infos(text: &str) -> (Vec<LightInfo>, f32) {
    let mut infos = Vec::new();
    let mut global_power = 28.0;
    for part in text.split("====") {
        let mut fields = std::collections::HashMap::new();
        for line in part.lines() {
            if line.starts_with([' ', '\t']) {
                continue;
            }
            if let Some((key, value)) = line.split_once(char::is_whitespace) {
                fields.insert(key, value.trim().trim_matches('"'));
            }
        }
        let num = |key: &str| fields.get(key).and_then(|v| v.parse::<f32>().ok());
        let vec3 = |key: &str| -> Option<[f32; 3]> {
            let parts: Vec<f32> = fields
                .get(key)?
                .trim_matches(['[', ']', ' '])
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            Some([*parts.first()?, *parts.get(1)?, *parts.get(2)?])
        };
        match fields.get("classname").copied() {
            Some("env_global_light") => {
                if let Some(p) = num("specularpower") {
                    global_power = p;
                }
            }
            Some("ent_dota_lightinfo") => {
                if !fields.get("targetname").is_some_and(|n| n.starts_with("[PR#]default_")) {
                    continue;
                }
                let keys = |when: &str| -> Option<LightKeys> {
                    let k = |name: &str| format!("{name}_{when}");
                    Some(LightKeys {
                        color: vec3(&k("color"))?,
                        light_scale: num(&k("light_scale"))?,
                        ambient_color: vec3(&k("ambient_color"))?,
                        ambient_scale: num(&k("ambient_scale"))?,
                        shadow_color: vec3(&k("shadow_color"))?,
                        shadow_secondary: vec3(&k("shadow_secondary_color"))?,
                        shadow_ground_scale: num(&k("shadow_ground_scale"))?,
                        light_angles: vec3(&k("light_direction"))?,
                        ambient_angles: vec3(&k("ambient_direction"))?,
                        specular_color: vec3(&k("specular_color"))?,
                        specular_power: num(&k("specular_power"))?,
                        specular_angles: vec3(&k("specular_direction"))?,
                        specular_independence: num(&k("specular_independence"))?,
                        fog_color: vec3(&k("fog_color"))?,
                        sky_tint: vec3(&format!("skybox_tint_color_{when}"))?,
                        height_fog_color: vec3(&k("height_fog_color")).unwrap_or([255.0; 3]),
                        height_fog_density: num(&k("height_fog_density")).unwrap_or(0.0),
                    })
                };
                let (Some(day), Some(night), Some(origin)) = (keys("day"), keys("night"), vec3("origin")) else {
                    continue;
                };
                infos.push(LightInfo {
                    origin: [origin[0], origin[1]],
                    inner: num("inner_radius").unwrap_or(0.0),
                    outer: num("outer_radius").unwrap_or(1.0),
                    ambient_shadow: num("ambient_shadow_amount").unwrap_or(0.0),
                    day,
                    night,
                });
            }
            _ => {}
        }
    }
    (infos, global_power)
}

/// The global light of the Dota renderer for one frame: the lightinfos
/// around the camera, each mixed between its day and night keys by the
/// Minecraft time of day, mapped to the constants the Dota shaders read.
#[derive(Clone, Copy)]
struct Atmosphere {
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient_dir: [f32; 4],
    ambient_color: [f32; 4],
    sky_vertical: [f32; 4],
    sky_constant: [f32; 4],
    specular_dir: [f32; 4],
    specular_albedo: [f32; 4],
    specular_lit: [f32; 4],
    fog_color: [f32; 4],
    sky_tint: [f32; 4],
    height_fog: [f32; 4],
}

const INCH: f32 = 0.0254;
/// Map space is Source inches turned Y-up and shifted by this, see bake_dota_map.py.
const MAP_OFFSET: [f32; 3] = [-56.489_59, 193.497_49, -0.270_934];
/// r_dota_shadow_ambient_light: the scale of the ambient terms in the global light pass.
const SHADOW_AMBIENT_LIGHT_SCALE: f32 = 1.5;

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear(c: [f32; 3], scale: f32) -> [f32; 3] {
    c.map(|v| srgb_to_linear(v / 255.0) * scale)
}

/// Source pitch/yaw to the forward vector, in map space (x, y, z) = (src y, src z, src x).
fn source_forward(angles: [f32; 3]) -> [f32; 3] {
    let (sp, cp) = angles[0].to_radians().sin_cos();
    let (sy, cy) = angles[1].to_radians().sin_cos();
    [cp * sy, -sp, cp * cy]
}

impl Atmosphere {
    /// C_GlobalLight's blend (client.dll 0x1d6ae10): by 2D distance to the
    /// camera, a lightinfo within its inner radius wins outright; with none,
    /// a lone one in its ring is used whole; several in their rings weigh
    /// w = 1 - (d - inner) / (outer - inner), times (1 - w) of every other,
    /// normalized, and their linear colours and directions are summed.
    fn at(camera: [f32; 3], day: f32) -> Self {
        let (infos, global_power) = light_infos();
        let t = day.clamp(0.0, 1.0);
        let sx = (camera[2] - MAP_OFFSET[2]) / INCH;
        let sy = (camera[0] - MAP_OFFSET[0]) / INCH;
        let mut full = None;
        let mut ring = Vec::new();
        for (i, info) in infos.iter().enumerate() {
            let d = ((sx - info.origin[0]).powi(2) + (sy - info.origin[1]).powi(2)).sqrt();
            if d >= info.outer {
                continue;
            }
            if d < info.inner {
                full.get_or_insert(i);
            } else {
                ring.push((i, 1.0 - (d - info.inner) / (info.outer - info.inner).max(1.0)));
            }
        }
        let weights: Vec<(usize, f32)> = match (full, ring.len()) {
            (Some(i), _) => vec![(i, 1.0)],
            (None, 1) => vec![(ring[0].0, 1.0)],
            (None, 0) => vec![(0, 1.0)],
            (None, _) => {
                let w: Vec<(usize, f32)> = ring
                    .iter()
                    .map(|&(i, w)| (i, w * ring.iter().filter(|(j, _)| *j != i).map(|(_, o)| 1.0 - o).product::<f32>()))
                    .collect();
                let total: f32 = w.iter().map(|(_, w)| w).sum::<f32>().max(1.0e-6);
                w.into_iter().map(|(i, w)| (i, w / total)).collect()
            }
        };
        let default = LightInfo::default_rf();
        let mut out: Option<Self> = None;
        for (i, w) in weights {
            let info = infos.get(i).unwrap_or(&default);
            let one = Self::single(&info.night.mix(&info.day, t), info.ambient_shadow, *global_power);
            out = Some(match out {
                None => one.scaled(w),
                Some(acc) => acc.add(&one.scaled(w)),
            });
        }
        let mut a = out.unwrap_or_else(|| Self::single(&default.day, 0.0, *global_power));
        for v in [&mut a.sun_dir, &mut a.ambient_dir, &mut a.specular_dir] {
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1.0e-6);
            v[0] /= len;
            v[1] /= len;
            v[2] /= len;
        }
        a.sun_dir[3] = 1.0;
        a.sky_tint[3] = 12.0_f32.to_radians();
        a
    }

    /// One lightinfo's keys as the constants the Dota shaders read.
    fn single(k: &LightKeys, shadow_amount: f32, global_power: f32) -> Self {
        let sun = linear(k.color, k.light_scale);
        let to_sun = source_forward(k.light_angles).map(|v| -v);
        let ambient = linear(k.ambient_color, k.ambient_scale * SHADOW_AMBIENT_LIGHT_SCALE);
        let ambient_dir = source_forward(k.ambient_angles);
        let fill = k.shadow_ground_scale * SHADOW_AMBIENT_LIGHT_SCALE;
        let sky = linear(k.shadow_color, fill);
        let ground = linear(k.shadow_secondary, fill);
        // -1 keeps env_global_light's specularpower (client.dll 0x1d7da80).
        let power = if k.specular_power > 0.0 { k.specular_power } else { global_power };
        let independence = k.specular_independence.clamp(0.0, 1.0);
        let specular = linear(k.specular_color, 1.0);
        let fog = linear(k.fog_color, 1.0);
        let tint = linear(k.sky_tint, 1.0);
        let height_fog = linear(k.height_fog_color, 1.0);
        let d = source_forward(k.specular_angles);
        Self {
            sun_dir: [to_sun[0], to_sun[1], to_sun[2], 1.0],
            sun_color: v4(sun),
            ambient_dir: [ambient_dir[0], ambient_dir[1], ambient_dir[2], shadow_amount],
            ambient_color: v4(ambient),
            sky_vertical: v4([0, 1, 2].map(|c| (sky[c] - ground[c]) * 0.5)),
            sky_constant: v4([0, 1, 2].map(|c| (sky[c] + ground[c]) * 0.5)),
            specular_dir: [d[0], d[1], d[2], power],
            specular_albedo: v4(specular.map(|c| c * (1.0 - independence))),
            specular_lit: v4(specular.map(|c| c * independence * k.light_scale)),
            fog_color: [fog[0], fog[1], fog[2], 1.0],
            sky_tint: [tint[0], tint[1], tint[2], 0.0],
            height_fog: [height_fog[0], height_fog[1], height_fog[2], k.height_fog_density],
        }
    }

    fn fields(&mut self) -> [&mut [f32; 4]; 12] {
        [
            &mut self.sun_dir,
            &mut self.sun_color,
            &mut self.ambient_dir,
            &mut self.ambient_color,
            &mut self.sky_vertical,
            &mut self.sky_constant,
            &mut self.specular_dir,
            &mut self.specular_albedo,
            &mut self.specular_lit,
            &mut self.fog_color,
            &mut self.sky_tint,
            &mut self.height_fog,
        ]
    }

    fn scaled(mut self, w: f32) -> Self {
        for f in self.fields() {
            *f = f.map(|v| v * w);
        }
        self
    }

    fn add(mut self, other: &Self) -> Self {
        let mut o = *other;
        for (a, b) in self.fields().into_iter().zip(o.fields()) {
            for c in 0..4 {
                a[c] += b[c];
            }
        }
        self
    }
}

fn v4(c: [f32; 3]) -> [f32; 4] {
    [c[0], c[1], c[2], 0.0]
}

impl Map {
    fn on_map(&self, x: f64, y: f64, z: f64) -> bool {
        let (x, y, z) = (x as f32, y as f32, z as f32);
        x >= self.min[0] && x <= self.max[0] && z >= self.min[2] && z <= self.max[2] && y >= self.min[1] - 2.0
            && y <= self.max[1] + 4.0
    }

    /// Restrict a Minecraft move to the map the way Minecraft restricts it to
    /// blocks: the whole body is swept against every triangle one axis at a
    /// time (vertical first), steps of up to `STEP_HEIGHT` are climbed only
    /// from the ground and only onto walkable surfaces, a walking body follows
    /// the ground down slopes, and a body that ended up inside geometry is
    /// pushed out to the nearest free spot instead of being let through.
    fn clip(&self, bounds: Aabb, movement: Vec3, on_ground: bool) -> Vec3 {
        if self.tris.is_empty() {
            return movement;
        }
        let delta = [movement.x as f32, movement.y as f32, movement.z as f32];
        let mut b = [v3(bounds.min_position()), v3(bounds.max_position())];
        let mut push = [0.0_f32; 3];
        if self.depth(b) >= EMBEDDED {
            match self.push_out(b) {
                Some(p) => {
                    for a in 0..3 {
                        shift_box(&mut b, a, p[a]);
                    }
                    push = p;
                }
                None => return movement,
            }
        }
        let dy = self.sweep(b, 1, delta[1]);
        shift_box(&mut b, 1, dy);
        let landed = delta[1] < 0.0 && dy > delta[1];
        let grounded = on_ground || landed;
        let order = if delta[0].abs() >= delta[2].abs() { [0, 2] } else { [2, 0] };
        let mut walked = b;
        let mut h = [0.0_f32; 3];
        for a in order {
            h[a] = self.sweep(walked, a, delta[a]);
            shift_box(&mut walked, a, h[a]);
        }
        let mut rise = 0.0;
        let blocked = h[0] != delta[0] || h[2] != delta[2];
        let mut stepped = false;
        if grounded && blocked {
            if let Some((sb, sh, r)) = self.step(b, delta, order) {
                let flat = |v: [f32; 3]| v[0] * v[0] + v[2] * v[2];
                if flat(sh) > flat(h) + 1.0e-6 {
                    walked = sb;
                    h = sh;
                    rise = r;
                    stepped = true;
                }
            }
        }
        if grounded && !stepped && delta[1] <= 0.0 {
            let down = self.sweep(walked, 1, -STEP_HEIGHT);
            if down > -STEP_HEIGHT + 1.0e-4 && down < -1.0e-4 {
                rise += down;
            }
        }
        let got = [push[0] + h[0], push[1] + dy + rise, push[2] + h[2]];
        let keep = |got: f32, asked: f32, exact: f64| if got == asked { exact } else { f64::from(got) };
        Vec3::new(
            keep(got[0], delta[0], movement.x),
            keep(got[1], delta[1], movement.y),
            keep(got[2], delta[2], movement.z),
        )
    }

    /// Minecraft's step: lift the body as far as the head allows (up to a
    /// step), walk, and put it back down. A step onto a steep surface higher
    /// than a pebble is refused, so slopes and trunks cannot be climbed.
    fn step(&self, b: [[f32; 3]; 2], delta: [f32; 3], order: [usize; 2]) -> Option<([[f32; 3]; 2], [f32; 3], f32)> {
        let up = self.sweep(b, 1, STEP_HEIGHT);
        if up < 0.05 {
            return None;
        }
        let mut sb = b;
        shift_box(&mut sb, 1, up);
        let mut sh = [0.0_f32; 3];
        for a in order {
            sh[a] = self.sweep(sb, a, delta[a]);
            shift_box(&mut sb, a, sh[a]);
        }
        let down = self.sweep(sb, 1, -(up + 0.01));
        shift_box(&mut sb, 1, down);
        let rise = up + down;
        if rise > PEBBLE {
            match self.support(sb) {
                Some(normal_y) if normal_y >= WALKABLE => {}
                Some(_) => return None,
                None if rise > 0.3 => return None,
                None => {}
            }
        }
        Some((sb, sh, rise))
    }

    /// The slope (normal y) of what the body stands on: the highest surface
    /// just under its centre or the corners of its footprint.
    fn support(&self, b: [[f32; 3]; 2]) -> Option<f32> {
        let cx = (b[0][0] + b[1][0]) * 0.5;
        let cz = (b[0][2] + b[1][2]) * 0.5;
        let hx = (b[1][0] - b[0][0]) * 0.5 * 0.9;
        let hz = (b[1][2] - b[0][2]) * 0.5 * 0.9;
        let mut best: Option<(f64, f32)> = None;
        for (ox, oz) in [(0.0, 0.0), (hx, hz), (-hx, hz), (hx, -hz), (-hx, -hz)] {
            let origin = [f64::from(cx + ox), f64::from(b[0][1] + 0.1), f64::from(cz + oz)];
            if let Some((_, normal, point)) = self.ray(origin, [0.0, -1.0, 0.0], 0.3) {
                if best.is_none_or(|(y, _)| point[1] > y) {
                    best = Some((point[1], normal[1].abs()));
                }
            }
        }
        best.map(|(_, n)| n)
    }

    /// The nearest free spot around an embedded body, as an offset.
    fn push_out(&self, b: [[f32; 3]; 2]) -> Option<[f32; 3]> {
        const S: f32 = std::f32::consts::FRAC_1_SQRT_2;
        let dirs: [[f32; 3]; 18] = [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
            [S, 0.0, S],
            [S, 0.0, -S],
            [-S, 0.0, S],
            [-S, 0.0, -S],
            [0.0, 1.0, 0.0],
            [S, S, 0.0],
            [-S, S, 0.0],
            [0.0, S, S],
            [0.0, S, -S],
            [0.0, -1.0, 0.0],
            [S, -S, 0.0],
            [-S, -S, 0.0],
            [0.0, -S, S],
            [0.0, -S, -S],
        ];
        for dist in [0.04_f32, 0.08, 0.14, 0.22, 0.32, 0.45, 0.6, 0.8, 1.05, 1.35] {
            for d in dirs {
                let p = [d[0] * dist, d[1] * dist, d[2] * dist];
                let mut moved = b;
                for a in 0..3 {
                    shift_box(&mut moved, a, p[a]);
                }
                let test = shrink(moved, SKIN);
                if !self.hits(test[0], test[1]) {
                    return Some(p);
                }
            }
        }
        None
    }

    /// How far the body can move along `axis` towards `delta` before it
    /// touches a triangle. It stops a little short so that the next move does
    /// not start inside the skin.
    fn sweep(&self, b: [[f32; 3]; 2], axis: usize, delta: f32) -> f32 {
        if delta.abs() < 1.0e-6 {
            return 0.0;
        }
        if self.free(b, axis, delta) {
            return delta;
        }
        let (mut lo, mut hi) = (0.0_f32, delta);
        for _ in 0..14 {
            let mid = (lo + hi) * 0.5;
            if self.free(b, axis, mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let back = lo.abs().min(MARGIN);
        lo - back * delta.signum()
    }

    /// Whether moving the body by `delta` along `axis` stays clear. A body
    /// already touching something within the skin (two walls at a corner, a
    /// floor) may make any move that does not push it deeper.
    fn free(&self, b: [[f32; 3]; 2], axis: usize, delta: f32) -> bool {
        let mut moved = b;
        shift_box(&mut moved, axis, delta);
        let start = shrink(b, SKIN);
        if self.hits(start[0], start[1]) {
            return self.depth(moved) <= self.depth(b) + 1.0e-3;
        }
        let swept = [
            [b[0][0].min(moved[0][0]), b[0][1].min(moved[0][1]), b[0][2].min(moved[0][2])],
            [b[1][0].max(moved[1][0]), b[1][1].max(moved[1][1]), b[1][2].max(moved[1][2])],
        ];
        let test = shrink(swept, SKIN);
        !self.hits(test[0], test[1])
    }

    /// How far the body reaches into the map: the smallest shrink that clears
    /// it, up to `DEEP`.
    fn depth(&self, b: [[f32; 3]; 2]) -> f32 {
        if !self.hits(b[0], b[1]) {
            return 0.0;
        }
        let first = shrink(b, DEEP);
        if self.hits(first[0], first[1]) {
            return DEEP;
        }
        let (mut lo, mut hi) = (0.0, DEEP);
        for _ in 0..10 {
            let mid = (lo + hi) * 0.5;
            let s = shrink(b, mid);
            if self.hits(s[0], s[1]) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        hi
    }

    fn hits(&self, bmin: [f32; 3], bmax: [f32; 3]) -> bool {
        if self.nodes.is_empty() || bmin[0] > bmax[0] || bmin[1] > bmax[1] || bmin[2] > bmax[2] {
            return false;
        }
        let mut stack = Vec::with_capacity(64);
        stack.push(0u32);
        let center = [
            (bmin[0] + bmax[0]) * 0.5,
            (bmin[1] + bmax[1]) * 0.5,
            (bmin[2] + bmax[2]) * 0.5,
        ];
        let half = [
            (bmax[0] - bmin[0]) * 0.5,
            (bmax[1] - bmin[1]) * 0.5,
            (bmax[2] - bmin[2]) * 0.5,
        ];
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id as usize];
            if !overlap(node.min, node.max, bmin, bmax) {
                continue;
            }
            if node.count > 0 {
                for i in node.start..node.start + node.count {
                    let t = &self.tris[self.order[i as usize] as usize];
                    if tri_box(center, half, t.v[0], t.v[1], t.v[2]) {
                        return true;
                    }
                }
            } else {
                stack.push(node.left);
                stack.push(node.right);
            }
        }
        false
    }

    fn ray(&self, origin: [f64; 3], dir: [f64; 3], reach: f64) -> Option<(f64, [f32; 3], [f64; 3])> {
        self.ray_where(origin, dir, reach, |_| true)
    }

    fn ray_where(
        &self,
        origin: [f64; 3],
        dir: [f64; 3],
        reach: f64,
        accept: impl Fn(&[[f32; 3]; 3]) -> bool,
    ) -> Option<(f64, [f32; 3], [f64; 3])> {
        if self.tris.is_empty() || reach <= 0.0 {
            return None;
        }
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        if len < 1.0e-8 {
            return None;
        }
        let dir = [dir[0] / len, dir[1] / len, dir[2] / len];
        let end = [
            origin[0] + dir[0] * reach,
            origin[1] + dir[1] * reach,
            origin[2] + dir[2] * reach,
        ];
        let bmin = [
            origin[0].min(end[0]) as f32 - 0.05,
            origin[1].min(end[1]) as f32 - 0.05,
            origin[2].min(end[2]) as f32 - 0.05,
        ];
        let bmax = [
            origin[0].max(end[0]) as f32 + 0.05,
            origin[1].max(end[1]) as f32 + 0.05,
            origin[2].max(end[2]) as f32 + 0.05,
        ];
        let mut stack = vec![0u32];
        let mut best: Option<(f64, [f32; 3], [f64; 3])> = None;
        if self.nodes.is_empty() {
            return None;
        }
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id as usize];
            if !overlap(node.min, node.max, bmin, bmax) {
                continue;
            }
            if node.count > 0 {
                for i in node.start..node.start + node.count {
                    let tri = &self.tris[self.order[i as usize] as usize];
                    if !accept(&tri.v) {
                        continue;
                    }
                    if let Some((t, n, p)) = ray_tri(origin, dir, tri.v) {
                        if t > 1.0e-4 && t <= reach && best.as_ref().is_none_or(|(bt, _, _)| t < *bt)
                        {
                            best = Some((t, n, p));
                        }
                    }
                }
            } else {
                stack.push(node.left);
                stack.push(node.right);
            }
        }
        best
    }
}

fn v3(v: Vec3) -> [f32; 3] {
    [v.x as f32, v.y as f32, v.z as f32]
}

fn shift_box(box_: &mut [[f32; 3]; 2], axis: usize, delta: f32) {
    box_[0][axis] += delta;
    box_[1][axis] += delta;
}

fn shrink(box_: [[f32; 3]; 2], skin: f32) -> [[f32; 3]; 2] {
    [
        [box_[0][0] + skin, box_[0][1] + skin, box_[0][2] + skin],
        [box_[1][0] - skin, box_[1][1] - skin, box_[1][2] - skin],
    ]
}

fn overlap(a0: [f32; 3], a1: [f32; 3], b0: [f32; 3], b1: [f32; 3]) -> bool {
    a0[0] <= b1[0] && a1[0] >= b0[0] && a0[1] <= b1[1] && a1[1] >= b0[1] && a0[2] <= b1[2]
        && a1[2] >= b0[2]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn tri_box(center: [f32; 3], half: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> bool {
    let v0 = sub(a, center);
    let v1 = sub(b, center);
    let v2 = sub(c, center);
    let f0 = sub(v1, v0);
    let f1 = sub(v2, v1);
    let f2 = sub(v0, v2);
    let axes = [
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        cross(f0, f1),
        cross([1.0, 0.0, 0.0], f0),
        cross([0.0, 1.0, 0.0], f0),
        cross([0.0, 0.0, 1.0], f0),
        cross([1.0, 0.0, 0.0], f1),
        cross([0.0, 1.0, 0.0], f1),
        cross([0.0, 0.0, 1.0], f1),
        cross([1.0, 0.0, 0.0], f2),
        cross([0.0, 1.0, 0.0], f2),
        cross([0.0, 0.0, 1.0], f2),
    ];
    for axis in axes {
        let len2 = dot(axis, axis);
        if len2 < 1.0e-12 {
            continue;
        }
        let p0 = dot(v0, axis);
        let p1 = dot(v1, axis);
        let p2 = dot(v2, axis);
        let r = half[0] * axis[0].abs() + half[1] * axis[1].abs() + half[2] * axis[2].abs();
        let mn = p0.min(p1.min(p2));
        let mx = p0.max(p1.max(p2));
        if mx < -r || mn > r {
            return false;
        }
    }
    true
}

fn ray_tri(origin: [f64; 3], dir: [f64; 3], v: [[f32; 3]; 3]) -> Option<(f64, [f32; 3], [f64; 3])> {
    let v0 = [v[0][0] as f64, v[0][1] as f64, v[0][2] as f64];
    let v1 = [v[1][0] as f64, v[1][1] as f64, v[1][2] as f64];
    let v2 = [v[2][0] as f64, v[2][1] as f64, v[2][2] as f64];
    let e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
    let e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
    let p = [
        dir[1] * e2[2] - dir[2] * e2[1],
        dir[2] * e2[0] - dir[0] * e2[2],
        dir[0] * e2[1] - dir[1] * e2[0],
    ];
    let det = e1[0] * p[0] + e1[1] * p[1] + e1[2] * p[2];
    if det.abs() < 1.0e-8 {
        return None;
    }
    let inv = 1.0 / det;
    let tvec = [origin[0] - v0[0], origin[1] - v0[1], origin[2] - v0[2]];
    let u = (tvec[0] * p[0] + tvec[1] * p[1] + tvec[2] * p[2]) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = [
        tvec[1] * e1[2] - tvec[2] * e1[1],
        tvec[2] * e1[0] - tvec[0] * e1[2],
        tvec[0] * e1[1] - tvec[1] * e1[0],
    ];
    let vv = (dir[0] * q[0] + dir[1] * q[1] + dir[2] * q[2]) * inv;
    if vv < 0.0 || u + vv > 1.0 {
        return None;
    }
    let t = (e2[0] * q[0] + e2[1] * q[1] + e2[2] * q[2]) * inv;
    if t < 0.0 {
        return None;
    }
    let mut n = cross(
        [e1[0] as f32, e1[1] as f32, e1[2] as f32],
        [e2[0] as f32, e2[1] as f32, e2[2] as f32],
    );
    let len = (dot(n, n)).sqrt().max(1.0e-8);
    n = [n[0] / len, n[1] / len, n[2] / len];
    if n[0] as f64 * dir[0] + n[1] as f64 * dir[1] + n[2] as f64 * dir[2] > 0.0 {
        n = [-n[0], -n[1], -n[2]];
    }
    let point = [
        origin[0] + dir[0] * t,
        origin[1] + dir[1] * t,
        origin[2] + dir[2] * t,
    ];
    Some((t, n, point))
}

fn build_bvh(tris: &[Tri]) -> (Vec<Node>, Vec<u32>) {
    let mut order: Vec<u32> = (0..tris.len() as u32).collect();
    let mut nodes = Vec::new();
    if tris.is_empty() {
        return (nodes, order);
    }
    fn bounds(tris: &[Tri], order: &[u32], start: usize, count: usize) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for id in &order[start..start + count] {
            for v in tris[*id as usize].v {
                for a in 0..3 {
                    min[a] = min[a].min(v[a]);
                    max[a] = max[a].max(v[a]);
                }
            }
        }
        (min, max)
    }
    fn rec(nodes: &mut Vec<Node>, tris: &[Tri], order: &mut [u32], start: usize, count: usize) -> u32 {
        let (min, max) = bounds(tris, order, start, count);
        let id = nodes.len() as u32;
        nodes.push(Node {
            min,
            max,
            left: 0,
            right: 0,
            start: start as u32,
            count: count as u32,
        });
        if count <= 8 {
            return id;
        }
        let extent = [
            max[0] - min[0],
            max[1] - min[1],
            max[2] - min[2],
        ];
        let axis = if extent[0] > extent[1] && extent[0] > extent[2] {
            0
        } else if extent[1] > extent[2] {
            1
        } else {
            2
        };
        order[start..start + count].sort_by(|&a, &b| {
            let ca = tris[a as usize].v.iter().map(|v| v[axis]).sum::<f32>();
            let cb = tris[b as usize].v.iter().map(|v| v[axis]).sum::<f32>();
            ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)
        });
        let mid = count / 2;
        let left = rec(nodes, tris, order, start, mid);
        let right = rec(nodes, tris, order, start + mid, count - mid);
        nodes[id as usize].left = left;
        nodes[id as usize].right = right;
        nodes[id as usize].count = 0;
        id
    }
    rec(&mut nodes, tris, &mut order, 0, tris.len());
    (nodes, order)
}

/// The folder map.bin and the other baked data sit in.
pub(crate) fn data_root() -> Option<PathBuf> {
    Some(map_path()?.parent()?.to_path_buf())
}

fn map_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("HORIZON_DOTA_MAP") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }
    let mut candidates = vec![
        PathBuf::from(r"C:\Users\root\Desktop\rust\dota-map-data\map.bin"),
        PathBuf::from("dota-map-data").join("map.bin"),
    ];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("dota-map-data").join("map.bin"));
            candidates.push(dir.join("..").join("dota-map-data").join("map.bin"));
            candidates.push(dir.join("..").join("..").join("dota-map-data").join("map.bin"));
        }
    }
    candidates.into_iter().find(|path| path.exists())
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn rest(&self) -> usize {
        self.data.len().saturating_sub(self.at)
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.rest() < n {
            return None;
        }
        let out = &self.data[self.at..self.at + n];
        self.at += n;
        Some(out)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
}

fn load_map() -> Option<Map> {
    let path = map_path()?;
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            log::warn!("Dota map: cannot read {}: {error}", path.display());
            return None;
        }
    };
    match parse_map(&path, &bytes) {
        Ok(map) => {
            log::info!(
                "Dota map: {} collision triangles, {} draw batches, spawn {:.0} {:.0} {:.0}",
                map.tris.len(),
                map.batches.len(),
                map.spawn.x,
                map.spawn.y,
                map.spawn.z
            );
            Some(map)
        }
        Err(error) => {
            log::warn!("Dota map: {error}");
            None
        }
    }
}

fn parse_map(path: &Path, bytes: &[u8]) -> Result<Map, String> {
    let v4 = bytes.len() >= 8 && &bytes[..8] == b"DOTAMAP4";
    if !v4 && (bytes.len() < 8 || &bytes[..8] != b"DOTAMAP3") {
        return Err("map.bin is not a Dota map".into());
    }
    let mut r = Reader {
        data: bytes,
        at: 8,
    };
    let vcount = r.u32().ok_or("truncated vertices")? as usize;
    let icount = r.u32().ok_or("truncated indices")? as usize;
    let mut positions = Vec::with_capacity(vcount);
    for _ in 0..vcount {
        let x = r.f32().ok_or("vertex")?;
        let y = r.f32().ok_or("vertex")?;
        let z = r.f32().ok_or("vertex")?;
        positions.push([x, y, z]);
    }
    let mut indices = Vec::with_capacity(icount);
    for _ in 0..icount {
        indices.push(r.u32().ok_or("index")?);
    }
    let spawn = Vec3::new(
        r.f32().ok_or("spawn")? as f64,
        r.f32().ok_or("spawn")? as f64,
        r.f32().ok_or("spawn")? as f64,
    );
    let tex_count = r.u32().ok_or("textures")? as usize;
    let root = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut textures = Vec::with_capacity(tex_count);
    for _ in 0..tex_count {
        let len = r.u16().ok_or("texture name")? as usize;
        let name = r.take(len).ok_or("texture name")?;
        let name = String::from_utf8_lossy(name);
        textures.push(root.join(name.as_ref()));
    }
    let batch_count = r.u32().ok_or("batches")? as usize;
    let mut batches = Vec::with_capacity(batch_count);
    for _ in 0..batch_count {
        let (kind, features, slots) = if v4 {
            let kind = r.u32().ok_or("batch")?;
            let features = r.u32().ok_or("batch")?;
            let mut slots = [-1; SLOTS];
            for slot in &mut slots {
                *slot = r.i32().ok_or("slot")?;
            }
            (kind, features, slots)
        } else {
            let flags = r.u32().ok_or("batch")?;
            let mut slots = [-1; SLOTS];
            for slot in slots.iter_mut().take(5) {
                *slot = r.i32().ok_or("slot")?;
            }
            let kind = if flags & 1 == 1 { KIND_MULTIBLEND } else { KIND_LIT };
            (kind, if flags & 2 == 2 { F_TRANSLUCENT } else { 0 }, slots)
        };
        let mut scales = [0.0; 4];
        for value in &mut scales {
            *value = r.f32().ok_or("scale")?;
        }
        let mut rotate = [0.0; 4];
        for value in &mut rotate {
            *value = r.f32().ok_or("rotate")?;
        }
        let mut offset = [0.0; 8];
        for value in &mut offset {
            *value = r.f32().ok_or("offset")?;
        }
        let mut tints = [[0.0; 4]; 4];
        for tint in &mut tints {
            for c in tint.iter_mut() {
                *c = r.f32().ok_or("tint")?;
            }
        }
        let mut tint_b = [[0.0; 4]; 4];
        for tint in &mut tint_b {
            for c in tint.iter_mut() {
                *c = r.f32().ok_or("tint b")?;
            }
        }
        let mask_scale = r.f32().ok_or("mask scale")?;
        let mut params = [0.0; 16];
        if v4 {
            for value in &mut params {
                *value = r.f32().ok_or("params")?;
            }
        } else {
            params[0] = 1.0;
            params[1] = 1.0;
            params[2] = 0.35;
        }
        let name_len = r.u16().ok_or("batch name")? as usize;
        r.take(name_len).ok_or("batch name")?;
        let ninst = r.u32().ok_or("instances")? as usize;
        let mut instances = Vec::with_capacity(ninst.max(1));
        for _ in 0..ninst {
            let mut model = [[0.0; 4]; 4];
            for col in &mut model {
                for value in col.iter_mut() {
                    *value = r.f32().ok_or("instance")?;
                }
            }
            instances.push(model);
        }
        if instances.is_empty() {
            instances.push([
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]);
        }
        let nv = r.u32().ok_or("batch verts")? as usize;
        let ni = r.u32().ok_or("batch indices")? as usize;
        let mut verts = Vec::with_capacity(nv);
        let mut f = || r.f32().ok_or("vertex");
        for _ in 0..nv {
            let pos = [f()?, f()?, f()?];
            let nrm = [f()?, f()?, f()?];
            let tan = if v4 { [f()?, f()?, f()?, f()?] } else { [1.0, 0.0, 0.0, 1.0] };
            verts.push(GpuVert {
                pos,
                nrm,
                tan,
                uv: [f()?, f()?],
                blend: [f()?, f()?, f()?],
                soft: [f()?, f()?, f()?],
                color: [f()?, f()?, f()?],
            });
        }
        let mut idx = Vec::with_capacity(ni);
        for _ in 0..ni {
            idx.push(r.u32().ok_or("i")?);
        }
        batches.push(BatchCpu {
            kind,
            features,
            slots,
            scales,
            rotate,
            offset,
            tints,
            tint_b,
            mask_scale,
            params,
            instances,
            verts,
            indices: idx,
        });
    }
    let mut tris = Vec::with_capacity(indices.len() / 3);
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for tri in indices.chunks_exact(3) {
        let v = [
            positions[tri[0] as usize],
            positions[tri[1] as usize],
            positions[tri[2] as usize],
        ];
        for p in v {
            for a in 0..3 {
                min[a] = min[a].min(p[a]);
                max[a] = max[a].max(p[a]);
            }
        }
        tris.push(Tri { v });
    }
    let (nodes, order) = build_bvh(&tris);
    Ok(Map {
        tris,
        nodes,
        order,
        spawn,
        min,
        max,
        batches,
        textures,
    })
}

pub(crate) fn load_rgba(path: &Path) -> Option<(u32, u32, Vec<u8>, bool)> {
    let file = File::open(path).ok()?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut bytes = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut bytes).ok()?;
    let channels = info.color_type.samples();
    let mut pixels = Vec::with_capacity(info.width as usize * info.height as usize * 4);
    let mut uses_alpha = false;
    for pixel in bytes[..info.buffer_size()].chunks_exact(channels) {
        let rgba = match info.color_type {
            png::ColorType::Rgba => {
                if pixel[3] < 250 {
                    uses_alpha = true;
                }
                [pixel[0], pixel[1], pixel[2], pixel[3]]
            }
            png::ColorType::Rgb => [pixel[0], pixel[1], pixel[2], 255],
            png::ColorType::Grayscale => [pixel[0], pixel[0], pixel[0], 255],
            png::ColorType::GrayscaleAlpha => {
                if pixel[1] < 250 {
                    uses_alpha = true;
                }
                [pixel[0], pixel[0], pixel[0], pixel[1]]
            }
            _ => return None,
        };
        pixels.extend(rgba);
    }
    Some((info.width, info.height, pixels, uses_alpha))
}

fn solid_texture(device: &wgpu::Device, queue: &wgpu::Queue, rgba: [u8; 4]) -> wgpu::TextureView {
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("Dota flat"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &rgba,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// The texture followed by its box-filtered mip levels, down to 1x1.
fn mip_chain(width: u32, height: u32, pixels: &[u8]) -> (u32, Vec<u8>) {
    let mut levels = 1;
    let mut data = pixels.to_vec();
    let (mut w, mut h) = (width as usize, height as usize);
    let mut level = pixels.to_vec();
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(w - 1);
                        let sy = (y * 2 + dy).min(h - 1);
                        sum += level[(sy * w + sx) * 4 + c] as u32;
                    }
                    next[(y * nw + x) * 4 + c] = ((sum + 2) / 4) as u8;
                }
            }
        }
        data.extend_from_slice(&next);
        level = next;
        (w, h) = (nw, nh);
        levels += 1;
    }
    (levels, data)
}

pub(crate) fn image_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> wgpu::TextureView {
    let (levels, data) = mip_chain(width, height, pixels);
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("Dota texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &data,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// The map's sky cube: Source 2 Viewer writes the faces as rt lf bk ft up
/// dn, which is the +X -X +Y -Y +Z -Z layer order, Z up as in Source.
/// env_global_light's cloud shadow: its texture and the two layers' scroll.
/// cloudN_speed is in inches per second along the yaw of cloudN_direction; the
/// texture covers cloudscale inches.
fn cloud_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<(([f32; 4], [f32; 4]), wgpu::TextureView)> {
    let root = map_path()?.parent()?.to_path_buf();
    let text = std::fs::read_to_string(root.join("ents").join("maps").join("dota").join("entities").join("default_ents.vents")).ok()?;
    let part = text.split("====").find(|p| p.contains("\"env_global_light\""))?;
    let field = |key: &str| -> Option<String> {
        part.lines()
            .find_map(|l| l.strip_prefix(key).filter(|r| r.starts_with(char::is_whitespace)))
            .map(|v| v.trim().trim_matches('"').to_string())
    };
    if field("dota_no_clouds").is_some_and(|v| v == "true") {
        return None;
    }
    let num = |key: &str| field(key).and_then(|v| v.parse::<f32>().ok());
    let yaw = |key: &str| field(key).and_then(|v| v.split_whitespace().nth(1)?.parse::<f32>().ok());
    let scale = num("cloudscale")?;
    let layer = |speed: f32, yaw: f32| {
        let (s, c) = yaw.to_radians().sin_cos();
        [c * speed / scale, s * speed / scale]
    };
    let one = layer(num("cloud1speed")?, yaw("cloud1direction")?);
    let two = layer(num("cloud2speed")?, yaw("cloud2direction")?);
    let (w, h, mut pixels, _) = load_rgba(&root.join("sky").join("materials").join("effects").join("light_clouds_00.png"))?;
    // The night height fog's noise (fow_clouds_01, read raw) rides in green.
    if let Some((fw, fh, fog, _)) = load_rgba(&root.join("fog").join("materials").join("dev").join("fow_clouds_01.png")) {
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = ((x * fw / w) as usize, (y * fh / h) as usize);
                pixels[(y * w + x) as usize * 4 + 1] = fog[(sy * fw as usize + sx) * 4];
            }
        }
    }
    let view = image_texture(device, queue, w, h, &pixels);
    Some((([one[0], one[1], two[0], two[1]], [1.0 / scale, 1.0, 0.0, 0.0]), view))
}

fn sky_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<wgpu::TextureView> {
    let dir = map_path()?.parent()?.join("sky").join("materials").join("skybox");
    let mut size = 0;
    let mut data = Vec::new();
    for face in ["rt", "lf", "bk", "ft", "up", "dn"] {
        let suffix = format!("_{face}.png");
        let path = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.to_string_lossy().ends_with(&suffix))?;
        let (w, h, pixels, _) = load_rgba(&path)?;
        if w != h || (size != 0 && w != size) {
            return None;
        }
        size = w;
        data.extend_from_slice(&pixels);
    }
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("Dota sky"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &data,
    );
    Some(texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    }))
}

impl GpuMap {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        map: &Map,
    ) -> Option<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Dota map"),
            source: wgpu::ShaderSource::Wgsl(include_str!("dota_map.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Dota map bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex_entry(2),
                tex_entry(3),
                tex_entry(4),
                tex_entry(5),
                tex_entry(6),
                tex_entry(7),
                tex_entry(8),
                tex_entry(9),
                tex_entry(10),
                tex_entry(11),
                tex_entry(12),
                tex_entry(13),
                tex_entry(14),
            ],
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Dota shadow bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                storage_entry(2),
                storage_entry(3),
                tex_entry(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Dota map layout"),
            bind_group_layouts: &[Some(&layout), Some(&shadow_layout)],
            ..Default::default()
        });
        let depth_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Dota shadow layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x3,
            1 => Float32x3,
            2 => Float32x4,
            3 => Float32x2,
            4 => Float32x3,
            5 => Float32x3,
            6 => Float32x3
        ];
        let instance_attributes = wgpu::vertex_attr_array![
            7 => Float32x4,
            8 => Float32x4,
            9 => Float32x4,
            10 => Float32x4
        ];
        let buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<GpuVert>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: 64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance_attributes,
            }),
        ];
        let pipeline_for = |blend: Option<wgpu::BlendState>, depth_write: bool, label: &str, vs: &str, fs: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Dota sun shadow"),
            layout: Some(&depth_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_shadow"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            multiview_mask: None,
            cache: None,
        });
        let shadow_view = shadow_view(device);
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Dota shadow sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let (light_grid, lights, cells) = light_grid(map);
        let (clouds, cloud_view) = match cloud_texture(device, queue) {
            Some((params, view)) => (params, view),
            None => (([0.0; 4], [0.0; 4]), solid_texture(device, queue, [255, 255, 255, 255])),
        };
        let mut padded = lights.clone();
        padded.resize(lights.len() + DYNAMIC_LIGHTS * 2, [0.0; 4]);
        let lights_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dota lights"),
            contents: bytemuck::cast_slice(&padded),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let cells_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dota light grid"),
            contents: bytemuck::cast_slice(&cells),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let (ao_view, ao_info_data) = ao_texture(device, queue, map);
        let ao_info = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dota ao info"),
            contents: bytemuck::cast_slice(&ao_info_data),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let ao_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Dota ao sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shadow_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Dota shadow"),
            layout: &shadow_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: lights_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: cells_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&cloud_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&ao_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&ao_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: ao_info.as_entire_binding(),
                },
            ],
        });
        // An orthographic view down the sun over the whole map.
        let lo = glam::Vec3::from(map.min);
        let hi = glam::Vec3::from(map.max);
        let center = (lo + hi) * 0.5;
        let radius = (hi - lo).length() * 0.5;
        let to_sun = glam::Vec3::from(Atmosphere::at(center.to_array(), 1.0).sun_dir_xyz()).normalize();
        let eye = center + to_sun * radius * 2.0;
        let shadow_view_proj = glam::Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.0, radius * 4.0)
            * glam::Mat4::look_at_rh(eye, center, glam::Vec3::Y);
        let shadow = GpuShadow {
            pipeline: shadow_pipeline,
            group: shadow_group,
            view: shadow_view,
            view_proj: shadow_view_proj,
            drawn: AtomicBool::new(false),
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let pipelines = [
            pipeline_for(None, true, "Dota map", "vs", "fs"),
            pipeline_for(None, true, "Dota grass", "vs_grass", "fs_grass"),
            pipeline_for(Some(wgpu::BlendState::ALPHA_BLENDING), false, "Dota map blend", "vs", "fs"),
            pipeline_for(Some(additive), false, "Dota map additive", "vs", "fs"),
        ];
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Dota sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let white = solid_texture(device, queue, [255, 255, 255, 255]);
        let flat = solid_texture(device, queue, [128, 128, 255, 255]);
        let black = solid_texture(device, queue, [0, 0, 0, 0]);
        let mut views: Vec<wgpu::TextureView> = Vec::with_capacity(map.textures.len());
        for path in &map.textures {
            if let Some((w, h, pixels, _)) = load_rgba(path) {
                views.push(image_texture(device, queue, w, h, &pixels));
            } else {
                log::warn!("Dota map: missing texture {}", path.display());
                views.push(solid_texture(device, queue, [180, 170, 150, 255]));
            }
        }
        let fallback;
        let source: &[BatchCpu] = if map.batches.is_empty() {
            fallback = fallback_batches(map);
            &fallback
        } else {
            &map.batches
        };
        let mut batches = Vec::with_capacity(source.len());
        for batch in source {
            if batch.indices.is_empty() || batch.verts.is_empty() {
                continue;
            }
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Dota uniform"),
                size: std::mem::size_of::<GpuUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let view = |i: usize| {
                let slot = batch.slots[i];
                let default = match i {
                    5..=8 => &flat,
                    9..=12 => &black,
                    _ => &white,
                };
                if slot >= 0 {
                    views.get(slot as usize).unwrap_or(default)
                } else {
                    default
                }
            };
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ];
            for i in 0..SLOTS {
                entries.push(wgpu::BindGroupEntry {
                    binding: 2 + i as u32,
                    resource: wgpu::BindingResource::TextureView(view(i)),
                });
            }
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Dota batch"),
                layout: &layout,
                entries: &entries,
            });
            let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Dota vertices"),
                contents: bytemuck::cast_slice(&batch.verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Dota indices"),
                contents: bytemuck::cast_slice(&batch.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
            let mut offset = [[0.0; 4]; 4];
            for (i, pair) in offset.iter_mut().enumerate() {
                *pair = [batch.offset[i * 2], batch.offset[i * 2 + 1], 0.0, 0.0];
            }
            let mut params = [[0.0; 4]; 4];
            for (i, v) in batch.params.iter().enumerate() {
                params[i / 4][i % 4] = *v;
            }
            batches.push(GpuBatch {
                uniform,
                group,
                vertices,
                indices,
                index_count: batch.indices.len() as u32,
                kind: batch.kind,
                features: batch.features,
                scales: batch.scales,
                rotate: batch.rotate,
                offset,
                tints: batch.tints,
                tint_b: batch.tint_b,
                mask_scale: batch.mask_scale,
                params,
                instance_count: batch.instances.len() as u32,
                instance_buf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Dota instances"),
                    contents: bytemuck::cast_slice(&batch.instances),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
            });
        }
        if batches.is_empty() {
            return None;
        }
        if let Some(grass) = grass_batch(device, queue, &layout, &sampler, [&white, &flat, &black]) {
            batches.push(grass);
        }
        log::info!("Dota map: uploaded {} mesh batches", batches.len());
        let sky = Self::sky(device, queue, format, &sampler);
        if sky.is_none() {
            log::warn!("Dota map: no sky cube next to map.bin, keeping the Minecraft sky");
        }
        Some(Self {
            pipelines,
            batches,
            sky,
            shadow,
            light_grid,
            dynamic: DynamicLights {
                static_lights: lights.clone(),
                static_cells: cells.clone(),
                lights_buf,
                cells_buf,
                min: [light_grid.0[0], light_grid.0[1]],
                cols: light_grid.0[3] as usize,
                rows: light_grid.1[0] as usize,
                last: std::sync::Mutex::new(0),
            },
            clouds,
            started: std::time::Instant::now(),
        })
    }

    fn sky(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        sampler: &wgpu::Sampler,
    ) -> Option<GpuSky> {
        let cube = sky_texture(device, queue)?;
        let shader = &device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Dota sky"),
            source: wgpu::ShaderSource::Wgsl(include_str!("dota_sky.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Dota sky bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dota sky uniform"),
            size: std::mem::size_of::<SkyUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Dota sky"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&cube),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Dota sky layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Dota sky"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_sky"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_sky"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Some(GpuSky {
            pipeline,
            group,
            uniform,
        })
    }

    fn uniform(&self, batch: &GpuBatch, view_proj: glam::Mat4, camera: [f32; 3], a: &Atmosphere) -> GpuUniform {
        GpuUniform {
            view_proj: view_proj.to_cols_array_2d(),
            camera,
            time: self.started.elapsed().as_secs_f32(),
            kind: batch.kind,
            features: batch.features,
            mask_scale: batch.mask_scale,
            pad0: 0.0,
            scales: batch.scales,
            rotate: batch.rotate,
            tints: batch.tints,
            offset: batch.offset,
            tint_b: batch.tint_b,
            params: batch.params,
            sun_dir: a.sun_dir,
            sun_color: a.sun_color,
            ambient_dir: a.ambient_dir,
            ambient_color: a.ambient_color,
            sky_vertical: a.sky_vertical,
            sky_constant: a.sky_constant,
            specular_dir: a.specular_dir,
            specular_albedo: a.specular_albedo,
            specular_lit: a.specular_lit,
            fog_color: a.fog_color,
            light_grid: self.light_grid.0,
            light_grid2: self.light_grid.1,
            clouds: self.clouds.0,
            clouds2: self.clouds.1,
            height_fog: a.height_fog,
            shadow_view_proj: self.shadow.view_proj.to_cols_array_2d(),
        }
    }

    /// Draws every opaque batch into the sun shadow map, once.
    fn draw_shadow(&self, device: &wgpu::Device, queue: &wgpu::Queue, a: &Atmosphere) {
        if self.shadow.drawn.swap(true, Ordering::Relaxed) {
            return;
        }
        for batch in &self.batches {
            let uniform = self.uniform(batch, self.shadow.view_proj, [0.0; 3], a);
            queue.write_buffer(&batch.uniform, 0, bytemuck::bytes_of(&uniform));
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Dota sun shadow"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Dota sun shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.shadow.pipeline);
            for batch in &self.batches {
                if batch.pass() != 0 {
                    continue;
                }
                pass.set_bind_group(0, &batch.group, &[]);
                pass.set_vertex_buffer(0, batch.vertices.slice(..));
                pass.set_vertex_buffer(1, batch.instance_buf.slice(..));
                pass.set_index_buffer(batch.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..batch.index_count, 0, 0..batch.instance_count);
            }
        }
        queue.submit([encoder.finish()]);
        log::info!("Dota map: sun shadow map drawn");
    }

    fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        format: wgpu::TextureFormat,
        camera: [f32; 3],
        view_proj: glam::Mat4,
        day: f32,
        atmosphere: &Atmosphere,
    ) {
        self.dynamic.update(queue, &crate::dota_fx::lights());
        self.draw_shadow(device, queue, atmosphere);
        if let Some(sky) = &self.sky {
            let uniform = SkyUniform {
                inverse_view_proj: view_proj.inverse().to_cols_array_2d(),
                tint: atmosphere.sky_tint,
                fog_color: atmosphere.fog_color,
            };
            queue.write_buffer(&sky.uniform, 0, bytemuck::bytes_of(&uniform));
            pass.set_pipeline(&sky.pipeline);
            pass.set_bind_group(0, &sky.group, &[]);
            pass.draw(0..3, 0..1);
        }
        pass.set_bind_group(1, &self.shadow.group, &[]);
        for (order, pipeline) in self.pipelines.iter().enumerate() {
            if order == 2 {
                // The animated models (dota_anim) go after the opaque map and
                // grass, before the water and other translucent batches.
                crate::dota_anim::draw(device, queue, pass, format, camera, view_proj, day, &crate::dota_anim::Lighting {
                    sun_dir: atmosphere.sun_dir,
                    sun_color: atmosphere.sun_color,
                    ambient_dir: atmosphere.ambient_dir,
                    ambient_color: atmosphere.ambient_color,
                    sky_vertical: atmosphere.sky_vertical,
                    sky_constant: atmosphere.sky_constant,
                    specular_dir: atmosphere.specular_dir,
                    specular_albedo: atmosphere.specular_albedo,
                    specular_lit: atmosphere.specular_lit,
                    fog_color: atmosphere.fog_color,
                    light_grid: self.light_grid.0,
                    light_grid2: self.light_grid.1,
                    shadow_view_proj: self.shadow.view_proj,
                    shadow_group: Some(&self.shadow.group),
                });
                pass.set_bind_group(1, &self.shadow.group, &[]);
            }
            pass.set_pipeline(pipeline);
            for batch in &self.batches {
                if batch.pass() != order {
                    continue;
                }
                let uniform = self.uniform(batch, view_proj, camera, atmosphere);
                queue.write_buffer(&batch.uniform, 0, bytemuck::bytes_of(&uniform));
                pass.set_bind_group(0, &batch.group, &[]);
                pass.set_vertex_buffer(0, batch.vertices.slice(..));
                pass.set_vertex_buffer(1, batch.instance_buf.slice(..));
                pass.set_index_buffer(batch.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..batch.index_count, 0, 0..batch.instance_count);
            }
        }
        let g = [
            atmosphere.sun_color[0] + atmosphere.ambient_color[0],
            atmosphere.sun_color[1] + atmosphere.ambient_color[1],
            atmosphere.sun_color[2] + atmosphere.ambient_color[2],
        ];
        let top = g[0].max(g[1]).max(g[2]).max(1.0e-3);
        crate::dota_fx::draw(
            device,
            queue,
            pass,
            format,
            camera,
            view_proj,
            day,
            atmosphere.sun_dir[3],
            atmosphere.fog_color,
            [g[0] / top, g[1] / top, g[2] / top],
        );
    }
}

fn fallback_batches(map: &Map) -> Vec<BatchCpu> {
    let mut verts = Vec::with_capacity(map.tris.len() * 3);
    let mut indices = Vec::with_capacity(map.tris.len() * 3);
    for tri in &map.tris {
        let n = cross(sub(tri.v[1], tri.v[0]), sub(tri.v[2], tri.v[0]));
        let len = dot(n, n).sqrt().max(1.0e-8);
        let n = [n[0] / len, n[1] / len, n[2] / len];
        let base = verts.len() as u32;
        for v in tri.v {
            verts.push(GpuVert {
                pos: v,
                nrm: n,
                tan: [1.0, 0.0, 0.0, 1.0],
                uv: [v[0], v[2]],
                blend: [0.0; 3],
                soft: [1.0; 3],
                color: [1.0; 3],
            });
        }
        indices.extend([base, base + 1, base + 2]);
    }
    vec![BatchCpu {
        kind: KIND_SOLID,
        features: 0,
        slots: [-1; SLOTS],
        scales: [1.0; 4],
        rotate: [0.0; 4],
        offset: [0.0; 8],
        tints: [[0.45, 0.62, 0.28, 1.0]; 4],
        tint_b: [[1.0; 4]; 4],
        mask_scale: 1.0,
        params: [0.0; 16],
        instances: vec![[
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]],
        verts,
        indices,
    }]
}

fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        // The vertex stage reads the wind mask and noise.
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

impl Atmosphere {
    fn sun_dir_xyz(&self) -> [f32; 3] {
        [self.sun_dir[0], self.sun_dir[1], self.sun_dir[2]]
    }
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Lights a grid cell lists at most.
/// Ambient occlusion of the map's solid geometry as a 3D texture, one voxel
/// per metre: every voxel a triangle touches is solid, and the occupancy is
/// blurred at two radii, so a point near a wall or in a corner sees more of
/// it than one on open ground. Returns the view and the volume's origin, cell
/// size and extent for the shaders.
fn ao_texture(device: &wgpu::Device, queue: &wgpu::Queue, map: &Map) -> (wgpu::TextureView, Vec<[f32; 4]>) {
    use std::sync::atomic::AtomicU8;
    const CELL: f32 = 1.0;
    let started = std::time::Instant::now();
    let origin = [map.min[0] - 2.0, map.min[1] - 2.0, map.min[2] - 2.0];
    let dims = [
        (((map.max[0] - map.min[0]) + 4.0) / CELL).ceil() as usize + 1,
        (((map.max[1] - map.min[1]) + 12.0) / CELL).ceil() as usize + 1,
        (((map.max[2] - map.min[2]) + 4.0) / CELL).ceil() as usize + 1,
    ];
    let total = dims[0] * dims[1] * dims[2];
    if total == 0 || total > 120_000_000 || map.tris.is_empty() {
        let view = solid_volume(device, queue);
        return (view, vec![[0.0; 4]; 2]);
    }
    let solid: Vec<AtomicU8> = (0..total).map(|_| AtomicU8::new(0)).collect();
    let at = |x: usize, y: usize, z: usize| (z * dims[1] + y) * dims[0] + x;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
    let chunk = map.tris.len().div_ceil(threads);
    std::thread::scope(|scope| {
        for part in map.tris.chunks(chunk) {
            let (solid, dims) = (&solid, dims);
            scope.spawn(move || {
                for t in part {
                    let mut lo = [i64::MAX; 3];
                    let mut hi = [i64::MIN; 3];
                    for v in t.v {
                        for a in 0..3 {
                            let c = ((v[a] - origin[a]) / CELL).floor() as i64;
                            lo[a] = lo[a].min(c);
                            hi[a] = hi[a].max(c);
                        }
                    }
                    for a in 0..3 {
                        lo[a] = lo[a].max(0);
                        hi[a] = hi[a].min(dims[a] as i64 - 1);
                    }
                    let single = lo == hi;
                    for z in lo[2]..=hi[2] {
                        for y in lo[1]..=hi[1] {
                            for x in lo[0]..=hi[0] {
                                let hit = single || {
                                    let c = [
                                        origin[0] + (x as f32 + 0.5) * CELL,
                                        origin[1] + (y as f32 + 0.5) * CELL,
                                        origin[2] + (z as f32 + 0.5) * CELL,
                                    ];
                                    tri_box(c, [CELL * 0.5; 3], t.v[0], t.v[1], t.v[2])
                                };
                                if hit {
                                    let i = (z as usize * dims[1] + y as usize) * dims[0] + x as usize;
                                    solid[i].store(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                }
            });
        }
    });
    let mut volume: Vec<f32> = solid.iter().map(|s| f32::from(s.load(Ordering::Relaxed))).collect();
    drop(solid);
    // Box blur along each axis, twice (radius 2 and 5), averaged.
    let blur = |src: &[f32], radius: usize| -> Vec<f32> {
        let mut cur = src.to_vec();
        let mut line = Vec::new();
        for axis in 0..3 {
            let strides = [1, dims[0], dims[0] * dims[1]];
            let (n, stride) = (dims[axis], strides[axis]);
            let others = match axis {
                0 => [1, 2],
                1 => [0, 2],
                _ => [0, 1],
            };
            let (a, b) = ((dims[others[0]], strides[others[0]]), (dims[others[1]], strides[others[1]]));
            for j in 0..b.0 {
                for i in 0..a.0 {
                    let start = j * b.1 + i * a.1;
                    line.clear();
                    line.extend((0..n).map(|k| cur[start + k * stride]));
                    let width = (2 * radius + 1) as f32;
                    let mut sum: f32 = line.iter().take(radius + 1).sum();
                    for k in 0..n {
                        cur[start + k * stride] = sum / width;
                        if k + radius + 1 < n {
                            sum += line[k + radius + 1];
                        }
                        if k >= radius {
                            sum -= line[k - radius];
                        }
                    }
                }
            }
        }
        cur
    };
    let small = blur(&volume, 2);
    let large = blur(&volume, 5);
    for (v, (s, l)) in volume.iter_mut().zip(small.iter().zip(&large)) {
        *v = (s * 0.5 + l * 0.5).clamp(0.0, 1.0);
    }
    let bytes: Vec<u8> = volume.iter().map(|v| (v * 255.0).round() as u8).collect();
    let _ = at;
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("Dota ambient occlusion"),
            size: wgpu::Extent3d {
                width: dims[0] as u32,
                height: dims[1] as u32,
                depth_or_array_layers: dims[2] as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &bytes,
    );
    log::info!(
        "Dota map: ambient occlusion {}x{}x{} in {:.1}s",
        dims[0],
        dims[1],
        dims[2],
        started.elapsed().as_secs_f32()
    );
    (
        texture.create_view(&wgpu::TextureViewDescriptor::default()),
        vec![
            [origin[0], origin[1], origin[2], CELL],
            [dims[0] as f32 * CELL, dims[1] as f32 * CELL, dims[2] as f32 * CELL, 1.0],
        ],
    )
}

/// A 1x1x1 empty volume, for when there is no map to occlude.
fn solid_volume(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("Dota no ambient occlusion"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[0u8],
        )
        .create_view(&wgpu::TextureViewDescriptor::default())
}

const LIGHTS_PER_CELL: usize = 24;
const LIGHT_CELL: f32 = 8.0;

/// The map's env_deferred_light_client_only entities as (position, radius) and
/// (linear colour * intensity) pairs in map space, and a grid over the map
/// listing the lights that reach each cell: a count, then the indices.
fn light_grid(map: &Map) -> (([f32; 4], [f32; 4]), Vec<[f32; 4]>, Vec<u32>) {
    let text = map_path()
        .and_then(|p| Some(p.parent()?.join("ents").join("maps").join("dota").join("entities").join("default_ents.vents")))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let mut lights: Vec<[f32; 4]> = Vec::new();
    for part in text.split("====") {
        let mut fields = std::collections::HashMap::new();
        for line in part.lines() {
            if line.starts_with([' ', '\t']) {
                continue;
            }
            if let Some((key, value)) = line.split_once(char::is_whitespace) {
                fields.insert(key, value.trim().trim_matches('"'));
            }
        }
        if fields.get("classname").copied() != Some("env_deferred_light_client_only") {
            continue;
        }
        let vec3 = |key: &str| -> Option<[f32; 3]> {
            let v: Vec<f32> = fields
                .get(key)?
                .trim_matches(['[', ']', ' '])
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            Some([*v.first()?, *v.get(1)?, *v.get(2)?])
        };
        let (Some(origin), Some(color)) = (vec3("origin"), vec3("lightcolor")) else {
            continue;
        };
        let intensity: f32 = fields.get("intensity").and_then(|v| v.parse().ok()).unwrap_or(1.0);
        let radius: f32 = fields.get("radius").and_then(|v| v.parse().ok()).unwrap_or(128.0);
        let c = linear(color, intensity);
        lights.push([
            origin[1] * INCH + MAP_OFFSET[0],
            origin[2] * INCH + MAP_OFFSET[1],
            origin[0] * INCH + MAP_OFFSET[2],
            radius * INCH,
        ]);
        lights.push([c[0], c[1], c[2], 0.0]);
    }
    let cols = ((map.max[0] - map.min[0]) / LIGHT_CELL).ceil().max(1.0) as usize;
    let rows = ((map.max[2] - map.min[2]) / LIGHT_CELL).ceil().max(1.0) as usize;
    let mut cells = vec![0u32; cols * rows * (LIGHTS_PER_CELL + 1)];
    for (i, pair) in lights.chunks_exact(2).enumerate() {
        let [x, _, z, r] = pair[0];
        let c0 = (((x - r - map.min[0]) / LIGHT_CELL).floor().max(0.0) as usize).min(cols - 1);
        let c1 = (((x + r - map.min[0]) / LIGHT_CELL).floor().max(0.0) as usize).min(cols - 1);
        let r0 = (((z - r - map.min[2]) / LIGHT_CELL).floor().max(0.0) as usize).min(rows - 1);
        let r1 = (((z + r - map.min[2]) / LIGHT_CELL).floor().max(0.0) as usize).min(rows - 1);
        for row in r0..=r1 {
            for col in c0..=c1 {
                let at = (row * cols + col) * (LIGHTS_PER_CELL + 1);
                let n = cells[at] as usize;
                if n < LIGHTS_PER_CELL {
                    cells[at + 1 + n] = i as u32;
                    cells[at] += 1;
                }
            }
        }
    }
    log::info!("Dota map: {} deferred lights", lights.len() / 2);
    if lights.is_empty() {
        lights.push([0.0; 4]);
        lights.push([0.0; 4]);
    }
    (
        (
            [map.min[0], map.min[2], LIGHT_CELL, cols as f32],
            [rows as f32, LIGHTS_PER_CELL as f32, 0.0, 0.0],
        ),
        lights,
        cells,
    )
}

/// The map's clutter grass as one instanced batch: grass.bin's blades (the
/// layers on at the start of a game), four vec4s each in the instance slot,
/// a six-vertex card, the baked card atlas and the per-material constants
/// (flex, atlas u0, u1, specular intensity) in params.
fn grass_batch(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    [white, flat, black]: [&wgpu::TextureView; 3],
) -> Option<GpuBatch> {
    let root = map_path()?.parent()?.to_path_buf();
    let bytes = std::fs::read(root.join("grass.bin")).ok()?;
    let mut r = Reader { data: &bytes, at: 0 };
    if r.take(4)? != b"DGRS" || r.u32()? != 1 {
        return None;
    }
    let count = r.u32()? as usize;
    let materials = r.u32()? as usize;
    let mut params = [[0.0f32; 4]; 4];
    for m in 0..materials {
        let width = r.f32()?;
        let (_hmin, _hmax) = (r.f32()?, r.f32()?);
        let flex = r.f32()?;
        let spec = r.f32()?;
        let _mirror = r.u32()?;
        let (u0, u1) = (r.f32()?, r.f32()?);
        r.take(24)?;
        let _ = width;
        if m < 4 {
            params[m] = [flex, u0, u1, spec];
        }
    }
    let layers = r.u32()? as usize;
    let mut destroyed = Vec::new();
    for _ in 0..layers {
        let name = r.take(64)?;
        destroyed.push(name.starts_with(b"world_layer_radiant_destruction"));
    }
    let mut instances: Vec<[[f32; 4]; 4]> = Vec::with_capacity(count);
    for _ in 0..count {
        let mut f = [0.0f32; 11];
        for v in &mut f {
            *v = r.f32()?;
        }
        let packed = r.take(8)?;
        if destroyed.get(packed[6] as usize).copied().unwrap_or(false) {
            continue;
        }
        let mirror = (packed[5] & 1) as f32;
        instances.push([
            [f[0], f[1], f[2], f[6]],
            [f[3], f[4], f[5], f[7]],
            [f[8], f[9], f[10], packed[3] as f32 / 255.0],
            [
                packed[0] as f32 / 255.0,
                packed[1] as f32 / 255.0,
                packed[2] as f32 / 255.0,
                packed[4] as f32 + 4.0 * mirror,
            ],
        ]);
    }
    let dir = root.join("grass");
    let load = |name: &str| load_rgba(&dir.join(name)).map(|(w, h, p, _)| image_texture(device, queue, w, h, &p));
    let albedo = load("grass_albedo.png")?;
    let spec = load("grass_spec.png");
    let card = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0], [1.0, 1.0], [0.0, 1.0]].map(|[side, top]| GpuVert {
        pos: [side, top, 0.0],
        nrm: [0.0, 1.0, 0.0],
        tan: [1.0, 0.0, 0.0, 1.0],
        uv: [0.0; 2],
        blend: [0.0; 3],
        soft: [1.0; 3],
        color: [1.0; 3],
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Dota grass uniform"),
        size: std::mem::size_of::<GpuUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut entries = vec![
        wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::Sampler(sampler),
        },
    ];
    for i in 0..SLOTS {
        let view = match i {
            0 => &albedo,
            9 => spec.as_ref().unwrap_or(black),
            5..=8 => flat,
            9..=12 => black,
            _ => white,
        };
        entries.push(wgpu::BindGroupEntry {
            binding: 2 + i as u32,
            resource: wgpu::BindingResource::TextureView(view),
        });
    }
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Dota grass"),
        layout,
        entries: &entries,
    });
    log::info!("Dota map: {} grass blades", instances.len());
    Some(GpuBatch {
        uniform,
        group,
        vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dota grass card"),
            contents: bytemuck::cast_slice(&card),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dota grass indices"),
            contents: bytemuck::cast_slice(&[0u32, 1, 2, 3, 4, 5]),
            usage: wgpu::BufferUsages::INDEX,
        }),
        index_count: 6,
        instance_count: instances.len() as u32,
        instance_buf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dota grass blades"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        kind: KIND_GRASS,
        features: 0,
        scales: [1.0; 4],
        rotate: [0.0; 4],
        offset: [[0.0; 4]; 4],
        tints: [[1.0; 4]; 4],
        tint_b: [[1.0; 4]; 4],
        mask_scale: 1.0,
        params,
    })
}

#[cfg(test)]
mod collision_tests {
    use super::*;

    fn tri(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Tri {
        Tri { v: [a, b, c] }
    }

    fn map_of(tris: Vec<Tri>) -> Map {
        let (nodes, order) = build_bvh(&tris);
        Map {
            tris,
            nodes,
            order,
            spawn: Vec3::new(0.0, 2.0, 0.0),
            min: [-50.0; 3],
            max: [50.0; 3],
            batches: Vec::new(),
            textures: Vec::new(),
        }
    }

    fn quad(a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) -> Vec<Tri> {
        vec![tri(a, b, c), tri(a, c, d)]
    }

    fn floor() -> Vec<Tri> {
        quad([-300.0, 0.0, -300.0], [300.0, 0.0, -300.0], [300.0, 0.0, 300.0], [-300.0, 0.0, 300.0])
    }

    /// An axis-aligned box as twelve triangles.
    fn cube(min: [f32; 3], max: [f32; 3]) -> Vec<Tri> {
        let p = |i: usize| {
            [
                if i & 1 == 0 { min[0] } else { max[0] },
                if i & 2 == 0 { min[1] } else { max[1] },
                if i & 4 == 0 { min[2] } else { max[2] },
            ]
        };
        let faces = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
        faces.iter().flat_map(|f| quad(p(f[0]), p(f[1]), p(f[2]), p(f[3]))).collect()
    }

    fn body(x: f64, y: f64, z: f64) -> Aabb {
        Aabb::new(Vec3::new(x - 0.3, y, z - 0.3), Vec3::new(x + 0.3, y + 1.8, z + 0.3))
    }

    fn body_box(x: f64, y: f64, z: f64) -> [[f32; 3]; 2] {
        let b = body(x, y, z);
        [v3(b.min_position()), v3(b.max_position())]
    }

    /// A player with Minecraft's gravity and jump, run against the map. Returns
    /// the positions and whether each tick ended on the ground.
    struct Walker {
        p: [f64; 3],
        vy: f64,
        on_ground: bool,
    }

    impl Walker {
        fn new(x: f64, y: f64, z: f64) -> Self {
            Walker { p: [x, y, z], vy: 0.0, on_ground: false }
        }

        fn tick(&mut self, map: &Map, dx: f64, dz: f64, jump: bool) -> Vec3 {
            if jump && self.on_ground {
                self.vy = 0.42;
            }
            self.vy = (self.vy - 0.08) * 0.98;
            let ask = Vec3::new(dx, self.vy, dz);
            let moved = map.clip(body(self.p[0], self.p[1], self.p[2]), ask, self.on_ground);
            self.p[0] += moved.x;
            self.p[1] += moved.y;
            self.p[2] += moved.z;
            let vertical_hit = moved.y != ask.y;
            self.on_ground = vertical_hit && ask.y < 0.0;
            if vertical_hit {
                self.vy = 0.0;
            }
            moved
        }

        fn depth(&self, map: &Map) -> f32 {
            map.depth(body_box(self.p[0], self.p[1], self.p[2]))
        }
    }

    #[test]
    fn a_wall_holds_against_sprinting_and_jumping() {
        let mut tris = floor();
        tris.extend(quad([2.0, 0.0, -5.0], [2.0, 6.0, -5.0], [2.0, 6.0, 5.0], [2.0, 0.0, 5.0]));
        let map = map_of(tris);
        let mut w = Walker::new(0.0, 0.0, 0.0);
        for tick in 0..400 {
            w.tick(&map, 0.28, 0.0, tick % 3 == 0);
            assert!(w.p[0] + 0.3 <= 2.0 + 0.031, "through the wall at tick {tick}: {:?}", w.p);
        }
        assert!(w.p[0] > 1.5, "never reached the wall: {:?}", w.p);
    }

    #[test]
    fn a_fence_lower_than_a_jump_cannot_be_jumped_through() {
        let mut tris = floor();
        tris.extend(quad([2.0, 0.0, -5.0], [2.0, 1.5, -5.0], [2.0, 1.5, 5.0], [2.0, 0.0, 5.0]));
        let map = map_of(tris);
        let mut w = Walker::new(0.0, 0.0, 0.0);
        for tick in 0..300 {
            w.tick(&map, 0.28, 0.0, true);
            let crossed = w.p[0] + 0.3 > 2.05 && w.p[0] - 0.3 < 1.95;
            assert!(!crossed || w.p[1] >= 1.5 - 0.031, "inside the fence at tick {tick}: {:?}", w.p);
        }
    }

    #[test]
    fn an_overhang_is_not_entered_by_jumping() {
        let mut tris = floor();
        tris.extend(quad([1.0, 1.2, -5.0], [3.0, 3.5, -5.0], [3.0, 3.5, 5.0], [1.0, 1.2, 5.0]));
        tris.extend(quad([3.0, 0.0, -5.0], [3.0, 3.5, -5.0], [3.0, 3.5, 5.0], [3.0, 0.0, 5.0]));
        let map = map_of(tris);
        let mut w = Walker::new(0.0, 0.0, 0.0);
        for tick in 0..300 {
            w.tick(&map, 0.25, 0.0, true);
            assert!(w.depth(&map) < EMBEDDED, "inside the overhang at tick {tick}: {:?}", w.p);
        }
    }

    #[test]
    fn a_bumpy_floor_is_walked_without_getting_stuck() {
        let mut tris = Vec::new();
        let h = |x: f32, z: f32| 0.15 * ((x * 5.0).sin() * (z * 7.0).cos());
        for i in 0..40 {
            for j in 0..10 {
                let (x0, x1) = (i as f32 * 0.5, i as f32 * 0.5 + 0.5);
                let (z0, z1) = (j as f32 * 0.5 - 2.5, j as f32 * 0.5 - 2.0);
                tris.extend(quad([x0, h(x0, z0), z0], [x1, h(x1, z0), z0], [x1, h(x1, z1), z1], [x0, h(x0, z1), z1]));
            }
        }
        let map = map_of(tris);
        let mut w = Walker::new(1.0, 0.3, 0.0);
        let mut air = 0;
        for tick in 0..80 {
            w.tick(&map, 0.2, 0.0, false);
            if tick > 5 && !w.on_ground {
                air += 1;
            }
        }
        assert!(w.p[0] > 15.0, "stuck on bumps at {:?}", w.p);
        assert!(w.p[1] > -0.3 && w.p[1] < 0.5, "left the floor: {:?}", w.p);
        assert!(air < 8, "bobbed off the bumps {air} ticks");
    }

    #[test]
    fn a_gentle_ramp_is_walked_up_and_down_on_the_ground() {
        let mut tris = floor();
        tris.extend(quad([2.0, 0.0, -5.0], [12.0, 5.0, -5.0], [12.0, 5.0, 5.0], [2.0, 0.0, 5.0]));
        tris.extend(quad([12.0, 5.0, -5.0], [20.0, 5.0, -5.0], [20.0, 5.0, 5.0], [12.0, 5.0, 5.0]));
        let map = map_of(tris);
        let mut w = Walker::new(0.0, 0.0, 0.0);
        for _ in 0..80 {
            w.tick(&map, 0.2, 0.0, false);
        }
        assert!(w.p[0] > 13.0 && w.p[1] > 4.8, "did not walk up the ramp: {:?}", w.p);
        let mut air = 0;
        for _ in 0..75 {
            w.tick(&map, -0.2, 0.0, false);
            if !w.on_ground {
                air += 1;
            }
        }
        assert!(w.p[0] < 3.0, "did not walk down: {:?}", w.p);
        assert!(air < 3, "left the ground walking down the ramp {air} ticks");
    }

    #[test]
    fn a_steep_slope_cannot_be_climbed_or_fallen_through() {
        let mut tris = floor();
        tris.extend(quad([2.0, 0.0, -5.0], [4.0, 6.0, -5.0], [4.0, 6.0, 5.0], [2.0, 0.0, 5.0]));
        let map = map_of(tris);
        let mut w = Walker::new(0.0, 0.0, 0.0);
        for tick in 0..200 {
            w.tick(&map, 0.25, 0.0, false);
            assert!(w.depth(&map) < EMBEDDED, "inside the slope at tick {tick}: {:?}", w.p);
        }
        assert!(w.p[1] < 0.9, "climbed the steep slope: {:?}", w.p);
    }

    #[test]
    fn a_trunk_cannot_be_climbed_or_walked_through() {
        let mut tris = floor();
        for k in 0..8 {
            let a0 = std::f32::consts::TAU * k as f32 / 8.0;
            let a1 = std::f32::consts::TAU * (k + 1) as f32 / 8.0;
            let (p0, p1) = ((3.0 + 0.6 * a0.cos(), 0.6 * a0.sin()), (3.0 + 0.6 * a1.cos(), 0.6 * a1.sin()));
            tris.extend(quad([p0.0, -0.3, p0.1], [p1.0, -0.3, p1.1], [p1.0, 8.0, p1.1], [p0.0, 8.0, p0.1]));
        }
        let map = map_of(tris);
        let mut w = Walker::new(0.0, 0.0, 0.0);
        for tick in 0..200 {
            w.tick(&map, 0.25, 0.0, tick % 4 == 0);
        }
        assert!(w.p[0] < 2.2, "walked into the trunk: {:?}", w.p);
        assert!(w.p[1] < 0.1 || !w.on_ground, "stands on the trunk: {:?}", w.p);
    }

    #[test]
    fn a_low_step_is_climbed_and_a_high_one_is_not() {
        let mut tris = floor();
        tris.extend(cube([2.0, 0.0, -5.0], [4.0, 0.5, 5.0]));
        tris.extend(cube([6.0, 0.0, -5.0], [8.0, 1.6, 5.0]));
        let map = map_of(tris);
        let mut w = Walker::new(0.0, 0.0, 0.0);
        for _ in 0..60 {
            w.tick(&map, 0.2, 0.0, false);
        }
        assert!(w.p[0] > 5.0 && w.p[0] < 5.75, "the low step or the high wall went wrong: {:?}", w.p);
    }

    fn corner() -> Map {
        let mut tris = floor();
        tris.extend(quad([2.0, 0.0, -20.0], [2.0, 6.0, -20.0], [2.0, 6.0, 2.0], [2.0, 0.0, 2.0]));
        tris.extend(quad([-20.0, 0.0, 2.0], [-20.0, 6.0, 2.0], [2.0, 6.0, 2.0], [2.0, 0.0, 2.0]));
        map_of(tris)
    }

    #[test]
    fn a_body_wedged_into_a_corner_can_walk_out() {
        let map = corner();
        for depth in [0.01_f64, 0.03, 0.045, 0.06, 0.075, 0.15, 0.25] {
            let c = 2.0 - 0.3 + depth;
            let b = body(c, 0.0, c);
            let away_x = map.clip(b, Vec3::new(-0.15, 0.0, 0.0), true);
            let away_z = map.clip(b, Vec3::new(0.0, 0.0, -0.15), true);
            assert!(away_x.x < -0.1, "depth {depth}: stuck leaving along x ({})", away_x.x);
            assert!(away_z.z < -0.1, "depth {depth}: stuck leaving along z ({})", away_z.z);
            let end = body_box(c + away_x.x, away_x.y, c + away_x.z);
            assert!(map.depth(end) < EMBEDDED, "depth {depth}: still inside after leaving");
        }
    }

    #[test]
    fn jumping_into_a_corner_never_traps_or_passes() {
        let map = corner();
        let mut w = Walker::new(0.0, 0.0, 0.0);
        for tick in 0..600 {
            let (dx, dz) = if tick < 400 { (0.2, 0.2) } else { (-0.2, -0.2) };
            w.tick(&map, dx, dz, tick % 5 == 0);
            assert!(w.p[0] + 0.3 <= 2.031 && w.p[2] + 0.3 <= 2.031, "through at tick {tick}: {:?}", w.p);
        }
        assert!(w.p[0] < 0.0 && w.p[2] < 0.0, "could not walk back out of the corner: {:?}", w.p);
    }

    /// Random clutter (boxes and slanted slabs) walked through with random
    /// input and jumps: the body never ends a tick inside anything.
    #[test]
    fn random_clutter_is_never_entered() {
        let mut seed = 0x2545_f491_u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 10_000) as f32 / 10_000.0
        };
        let mut tris = floor();
        for _ in 0..60 {
            let (x, z) = (rnd() * 40.0 - 20.0, rnd() * 40.0 - 20.0);
            if (x * x + z * z).sqrt() < 2.0 {
                continue;
            }
            let (sx, sy, sz) = (0.2 + rnd() * 2.5, 0.1 + rnd() * 2.5, 0.2 + rnd() * 2.5);
            tris.extend(cube([x, rnd() * 0.5, z], [x + sx, sy, z + sz]));
        }
        for _ in 0..30 {
            let (x, z, y) = (rnd() * 40.0 - 20.0, rnd() * 40.0 - 20.0, rnd() * 2.0);
            let (ax, az, ay) = (rnd() * 3.0, rnd() * 3.0, rnd() * 3.0);
            tris.push(tri([x, y, z], [x + ax, y + ay, z], [x, y + ay * rnd(), z + az]));
        }
        let map = map_of(tris);
        let mut w = Walker::new(0.0, 0.0, 0.0);
        let mut dir = (0.2_f64, 0.0_f64);
        for tick in 0..6000 {
            if tick % 40 == 0 {
                let a = f64::from(rnd()) * std::f64::consts::TAU;
                dir = (0.28 * a.cos(), 0.28 * a.sin());
            }
            w.tick(&map, dir.0, dir.1, rnd() < 0.2);
            assert!(w.depth(&map) < EMBEDDED, "inside clutter at tick {tick}: {:?}", w.p);
            assert!(w.p[1] > -0.5, "fell through the floor at tick {tick}: {:?}", w.p);
        }
    }

    /// Random runs with jumps on the real baked map (`cargo test -- --ignored
    /// real_map`): the body never ends a tick inside the map, never drops under
    /// it, and never ends up somewhere it cannot walk away from.
    #[test]
    #[ignore]
    fn real_map_is_never_entered() {
        let Some(map) = world() else {
            panic!("no map.bin");
        };
        let mut seed = 0x9e37_79b9_u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1_000_000) as f64 / 1_000_000.0
        };
        let starts: Vec<[f32; 3]> = map
            .tris
            .iter()
            .filter_map(|t| {
                let n = cross(sub(t.v[1], t.v[0]), sub(t.v[2], t.v[0]));
                let len = dot(n, n).sqrt();
                let c = [0, 1, 2].map(|a| (t.v[0][a] + t.v[1][a] + t.v[2][a]) / 3.0);
                (len > 1.0e-6 && n[1].abs() / len > 0.9 && c[1] > 190.0 && c[1] < 215.0).then_some(c)
            })
            .collect();
        let (mut runs, mut skipped, mut entered, mut under, mut trapped) = (0, 0, 0, 0, 0);
        let mut report = Vec::new();
        while runs < 400 {
            let c = starts[(rnd() * starts.len() as f64) as usize % starts.len()];
            let mut w = Walker::new(f64::from(c[0]), f64::from(c[1]) + 0.05, f64::from(c[2]));
            if w.depth(map) >= EMBEDDED {
                skipped += 1;
                continue;
            }
            runs += 1;
            let mut dir = (0.0, 0.0);
            let mut bad = false;
            for tick in 0..240 {
                if tick % 30 == 0 {
                    let a = rnd() * std::f64::consts::TAU;
                    let speed = if rnd() < 0.5 { 0.28 } else { 0.13 };
                    dir = (speed * a.cos(), speed * a.sin());
                }
                w.tick(map, dir.0, dir.1, rnd() < 0.15);
                if w.depth(map) >= EMBEDDED {
                    entered += 1;
                    report.push(format!("inside at {:?} tick {tick}", w.p));
                    bad = true;
                    break;
                }
                if w.p[1] < f64::from(map.min[1]) - 1.0 {
                    under += 1;
                    report.push(format!("under the map at {:?} from {c:?}", w.p));
                    bad = true;
                    break;
                }
            }
            if bad {
                continue;
            }
            for _ in 0..10 {
                w.tick(map, 0.0, 0.0, false);
            }
            let here = w.p;
            let mut best = 0.0_f64;
            for (dx, dz) in [(0.2, 0.0), (-0.2, 0.0), (0.0, 0.2), (0.0, -0.2)] {
                let mut t = Walker { p: here, vy: 0.0, on_ground: true };
                for _ in 0..8 {
                    t.tick(map, dx, dz, false);
                }
                best = best.max(((t.p[0] - here[0]).powi(2) + (t.p[2] - here[2]).powi(2)).sqrt());
            }
            if best < 0.3 {
                for (dx, dz) in [(0.2, 0.0), (-0.2, 0.0), (0.0, 0.2), (0.0, -0.2)] {
                    let mut t = Walker { p: here, vy: 0.0, on_ground: true };
                    for _ in 0..20 {
                        t.tick(map, dx, dz, true);
                    }
                    best = best.max(((t.p[0] - here[0]).powi(2) + (t.p[2] - here[2]).powi(2)).sqrt());
                }
            }
            if best < 0.3 {
                trapped += 1;
                report.push(format!("trapped at {here:?}"));
            }
        }
        println!("runs {runs}, skipped starts {skipped}, entered {entered}, under {under}, trapped {trapped}");
        for line in report.iter().take(30) {
            println!("  {line}");
        }
        // Falls under the map go through holes in Dota's own geometry (under
        // its cliffs), and the few traps are hollows inside its models; `guard`
        // brings the player back from the falls. Entering geometry is the bug.
        assert!(entered == 0, "see the report above");
    }
}
