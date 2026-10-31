//! Animated models of the Dota map: towers, banners, shrines, shopkeepers,
//! cattails and lily pads, and the ambient creatures (birds, fish, frogs...).
//!
//! `anim.bin` sits next to `map.bin` and is baked by tools/bake_dota_anim.py
//! from the player's own Dota install. It is read on a worker thread the first
//! time the map is drawn; textures stream in a few per frame and nothing is
//! drawn until all of them are on the GPU. Every frame the CPU samples one
//! bone palette per (model, clip, phase) into a storage buffer and each mesh is
//! drawn once, instanced and skinned, with the lighting of dota_map.wgsl.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use glam::{Mat4, Quat, Vec3};
use wgpu::util::DeviceExt;

/// The light dota_map computes for the frame (its `Atmosphere`), and its sun
/// shadow map and point lights.
pub struct Lighting<'a> {
    pub sun_dir: [f32; 4],
    pub sun_color: [f32; 4],
    pub ambient_dir: [f32; 4],
    pub ambient_color: [f32; 4],
    pub sky_vertical: [f32; 4],
    pub sky_constant: [f32; 4],
    pub specular_dir: [f32; 4],
    pub specular_albedo: [f32; 4],
    pub specular_lit: [f32; 4],
    pub fog_color: [f32; 4],
    /// origin x, origin z, cell size, columns (cell size 0: no point lights)
    pub light_grid: [f32; 4],
    /// rows, lights per cell
    pub light_grid2: [f32; 4],
    pub shadow_view_proj: Mat4,
    /// dota_map's group 1: sun shadow depth, comparison sampler, lights,
    /// light cells, all fragment-visible. None draws without them.
    pub shadow_group: Option<&'a wgpu::BindGroup>,
}

const KIND_SOLID: u32 = 3;
const F_TRANSLUCENT: u32 = 4;
const F_ADDITIVE: u32 = 8;
const SHOW_DAY: u32 = 1;
const SHOW_NIGHT: u32 = 2;
/// Material texture slots: colour, scroll speed, tint mask, normal, specular (or hero masks2), hero masks1.
const SLOTS: usize = 6;
/// Particle creatures (dota_fx) drawn per frame at most.
const CREATURES_MAX: usize = 384;
/// Bones reserved in the palette buffer for each of them.
const CREATURE_BONES: usize = 24;

/// A model a particle system puts in the world (a butterfly, a bat...): placed
/// each frame by dota_fx, animated from `time` on its clip (`clip` is the
/// last word of the clip's name in lower case: idle, flutter, anim).
#[derive(Clone, Debug)]
pub struct Creature {
    pub model: String,
    pub clip: String,
    pub time: f32,
    pub matrix: Mat4,
    pub tint: [f32; 3],
    pub fade: f32,
}

static CREATURES: Mutex<Vec<Creature>> = Mutex::new(Vec::new());

/// The particle models to draw this frame (replaces last frame's).
pub fn set_creatures(list: Vec<Creature>) {
    if let Ok(mut c) = CREATURES.lock() {
        *c = list;
    }
}
const FADE_SECONDS: f32 = 0.75;
const TEXTURES_PER_FRAME: usize = 4;

#[derive(Clone, Debug, PartialEq)]
struct Material {
    kind: u32,
    features: u32,
    slots: [i32; SLOTS],
    scales: [f32; 4],
    offset: [f32; 4],
    tint: [f32; 4],
    params: [f32; 16],
    mask_scale: f32,
}

impl Material {
    /// 0 opaque, 1 alpha blended, 2 additive.
    fn pass(&self) -> usize {
        if self.features & F_ADDITIVE != 0 {
            2
        } else if self.features & F_TRANSLUCENT != 0 {
            1
        } else {
            0
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Trs {
    t: Vec3,
    r: Quat,
    s: Vec3,
}

impl Trs {
    fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.s, self.r, self.t)
    }

    /// Linear blend, the rotation normalized and taken the short way round.
    fn lerp(&self, o: &Self, f: f32) -> Self {
        let mut r1 = o.r;
        if self.r.dot(r1) < 0.0 {
            r1 = -r1;
        }
        let r = Quat::from_xyzw(
            self.r.x + (r1.x - self.r.x) * f,
            self.r.y + (r1.y - self.r.y) * f,
            self.r.z + (r1.z - self.r.z) * f,
            self.r.w + (r1.w - self.r.w) * f,
        );
        let r = if r.length_squared() > 1.0e-12 { r.normalize() } else { self.r };
        Self {
            t: self.t.lerp(o.t, f),
            r,
            s: self.s.lerp(o.s, f),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct AnimVert {
    pos: [f32; 3],
    nrm: [f32; 3],
    tan: [f32; 4],
    uv: [f32; 2],
    joints: [u16; 4],
    weights: [f32; 4],
}

struct Mesh {
    material: u32,
    verts: Vec<AnimVert>,
    indices: Vec<u32>,
}

struct Model {
    #[cfg_attr(not(test), allow(dead_code))]
    name: String,
    parents: Vec<i32>,
    rest: Vec<Trs>,
    /// Nodes ordered so every parent comes before its children.
    order: Vec<usize>,
    bones: Vec<(u32, Mat4)>,
    meshes: Vec<Mesh>,
}

/// Per node: no keys (rest pose), one key (constant), or one per frame.
struct Clip {
    model: u32,
    #[cfg_attr(not(test), allow(dead_code))]
    name: String,
    fps: f32,
    frames: u32,
    tracks: Vec<(u32, u32)>,
    keys: Vec<Trs>,
}

impl Clip {
    fn duration(&self) -> f32 {
        if self.frames > 1 && self.fps > 0.0 {
            (self.frames - 1) as f32 / self.fps
        } else {
            0.0
        }
    }

    /// The two frames around a time and how far between them, looping.
    fn frame_at(&self, time: f32) -> (usize, usize, f32) {
        let duration = self.duration();
        if duration <= 0.0 {
            return (0, 0, 0.0);
        }
        let t = time.rem_euclid(duration);
        let f = t * self.fps;
        let i0 = (f.floor() as usize).min(self.frames as usize - 2);
        let frac = (f - i0 as f32).clamp(0.0, 1.0);
        (i0, i0 + 1, frac)
    }

    fn local(&self, node: usize, rest: &Trs, frame: (usize, usize, f32)) -> Trs {
        let Some(&(start, count)) = self.tracks.get(node) else {
            return *rest;
        };
        match count {
            0 => *rest,
            1 => self.keys[start as usize],
            n => {
                let n = n as usize;
                let a = self.keys[start as usize + frame.0.min(n - 1)];
                let b = self.keys[start as usize + frame.1.min(n - 1)];
                a.lerp(&b, frame.2)
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Instance {
    model: u32,
    clip: i32,
    phase: f32,
    flags: u32,
    tint: [f32; 3],
    matrix: Mat4,
}

struct AnimData {
    textures: Vec<PathBuf>,
    materials: Vec<Material>,
    models: Vec<Model>,
    clips: Vec<Clip>,
    instances: Vec<Instance>,
}

/// Whether an instance shows at this time of day: entities without
/// showatday/showatnight show always, and so do ambient creatures that set
/// neither.
fn shown(flags: u32, is_day: bool) -> bool {
    if flags & (SHOW_DAY | SHOW_NIGHT) == 0 {
        return true;
    }
    if is_day { flags & SHOW_DAY != 0 } else { flags & SHOW_NIGHT != 0 }
}

/// Moves a fade towards its target over FADE_SECONDS.
fn step_fade(fade: f32, target: bool, dt: f32) -> f32 {
    let step = dt / FADE_SECONDS;
    if target { (fade + step).min(1.0) } else { (fade - step).max(0.0) }
}

impl Model {
    /// Global node matrices in model space for a clip at a time (rest pose without a clip).
    fn globals(&self, clip: Option<&Clip>, time: f32, out: &mut Vec<Mat4>) {
        out.clear();
        out.resize(self.parents.len(), Mat4::IDENTITY);
        let frame = clip.map(|c| c.frame_at(time)).unwrap_or((0, 0, 0.0));
        for &node in &self.order {
            let local = match clip {
                Some(c) => c.local(node, &self.rest[node], frame),
                None => self.rest[node],
            }
            .matrix();
            let parent = self.parents[node];
            out[node] = if parent >= 0 { out[parent as usize] * local } else { local };
        }
    }

    /// Joint matrix of every bone: global node matrix times inverse bind.
    fn palette(&self, clip: Option<&Clip>, time: f32, scratch: &mut Vec<Mat4>, out: &mut Vec<[[f32; 4]; 4]>) {
        self.globals(clip, time, scratch);
        for (node, inverse_bind) in &self.bones {
            let global = scratch.get(*node as usize).copied().unwrap_or(Mat4::IDENTITY);
            out.push((global * *inverse_bind).to_cols_array_2d());
        }
    }
}

fn node_order(parents: &[i32]) -> Vec<usize> {
    let n = parents.len();
    let mut order = Vec::with_capacity(n);
    let mut placed = vec![false; n];
    for start in 0..n {
        let mut chain = Vec::new();
        let mut at = start as i32;
        while at >= 0 && (at as usize) < n && !placed[at as usize] && !chain.contains(&(at as usize)) {
            chain.push(at as usize);
            at = parents[at as usize];
        }
        for &node in chain.iter().rev() {
            placed[node] = true;
            order.push(node);
        }
    }
    order
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.data.len().saturating_sub(self.at) < n {
            return Err(format!("anim.bin truncated at byte {}", self.at));
        }
        let out = &self.data[self.at..self.at + n];
        self.at += n;
        Ok(out)
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn floats<const N: usize>(&mut self) -> Result<[f32; N], String> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = self.f32()?;
        }
        Ok(out)
    }
    fn string(&mut self) -> Result<String, String> {
        let len = self.u16()? as usize;
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }
    fn count(&mut self, item_bytes: usize) -> Result<usize, String> {
        let n = self.u32()? as usize;
        if n.saturating_mul(item_bytes) > self.data.len().saturating_sub(self.at) {
            return Err(format!("anim.bin count {n} too large at byte {}", self.at));
        }
        Ok(n)
    }
    fn trs(&mut self) -> Result<Trs, String> {
        let v: [f32; 10] = self.floats()?;
        Ok(Trs {
            t: Vec3::new(v[0], v[1], v[2]),
            r: Quat::from_xyzw(v[3], v[4], v[5], v[6]),
            s: Vec3::new(v[7], v[8], v[9]),
        })
    }
}

fn parse_anim(root: &Path, bytes: &[u8]) -> Result<AnimData, String> {
    if bytes.len() < 8 || &bytes[..8] != b"DOTAANM1" {
        return Err("anim.bin is not DOTAANM1".into());
    }
    let mut r = Reader { data: bytes, at: 8 };
    let n = r.count(2)?;
    let mut textures = Vec::with_capacity(n);
    for _ in 0..n {
        textures.push(root.join(r.string()?));
    }
    let n = r.count(4)?;
    let mut materials = Vec::with_capacity(n);
    for _ in 0..n {
        let kind = r.u32()?;
        let features = r.u32()?;
        let mut slots = [-1; SLOTS];
        for s in &mut slots {
            *s = r.i32()?;
        }
        materials.push(Material {
            kind,
            features,
            slots,
            scales: r.floats()?,
            offset: r.floats()?,
            tint: r.floats()?,
            params: r.floats()?,
            mask_scale: r.f32()?,
        });
    }
    let n = r.count(4)?;
    let mut models = Vec::with_capacity(n);
    for _ in 0..n {
        let name = r.string()?;
        let nodes = r.count(44)?;
        let mut parents = Vec::with_capacity(nodes);
        let mut rest = Vec::with_capacity(nodes);
        for _ in 0..nodes {
            let parent = r.i32()?;
            parents.push(if parent >= 0 && (parent as usize) < nodes { parent } else { -1 });
            rest.push(r.trs()?);
        }
        let nbones = r.count(68)?;
        let mut bones = Vec::with_capacity(nbones);
        for _ in 0..nbones {
            let node = r.u32()?;
            if node as usize >= nodes {
                return Err(format!("{name}: bone node {node} out of range"));
            }
            bones.push((node, Mat4::from_cols_array(&r.floats::<16>()?)));
        }
        let nmeshes = r.count(12)?;
        let mut meshes = Vec::with_capacity(nmeshes);
        for _ in 0..nmeshes {
            let material = r.u32()?;
            let nv = r.u32()? as usize;
            let ni = r.u32()? as usize;
            let raw = r.take(nv.checked_mul(std::mem::size_of::<AnimVert>()).ok_or("vertex count")?)?;
            let mut verts: Vec<AnimVert> = bytemuck::pod_collect_to_vec(raw);
            for v in &mut verts {
                for j in &mut v.joints {
                    if *j as usize >= nbones {
                        *j = 0;
                    }
                }
            }
            let raw = r.take(ni.checked_mul(4).ok_or("index count")?)?;
            let indices: Vec<u32> = bytemuck::pod_collect_to_vec(raw);
            if indices.iter().any(|&i| i as usize >= nv) || material as usize >= materials.len() {
                return Err(format!("{name}: bad mesh"));
            }
            meshes.push(Mesh { material, verts, indices });
        }
        models.push(Model {
            name,
            order: node_order(&parents),
            parents,
            rest,
            bones,
            meshes,
        });
    }
    let n = r.count(12)?;
    let mut clips = Vec::with_capacity(n);
    for _ in 0..n {
        let model = r.u32()?;
        let name = r.string()?;
        let fps = r.f32()?;
        let frames = r.u32()?.max(1);
        let Some(m) = models.get(model as usize) else {
            return Err(format!("clip {name}: model {model} out of range"));
        };
        let mut tracks = Vec::with_capacity(m.parents.len());
        let mut keys = Vec::new();
        for _ in 0..m.parents.len() {
            let count = r.u32()?;
            if count > frames.max(1) {
                return Err(format!("clip {name}: {count} keys for {frames} frames"));
            }
            tracks.push((keys.len() as u32, count));
            for _ in 0..count {
                keys.push(r.trs()?);
            }
        }
        clips.push(Clip {
            model,
            name,
            fps,
            frames,
            tracks,
            keys,
        });
    }
    let n = r.count(92)?;
    let mut instances = Vec::with_capacity(n);
    for _ in 0..n {
        let model = r.u32()?;
        let clip = r.i32()?;
        let phase = r.f32()?;
        let flags = r.u32()?;
        let tint = r.floats::<3>()?;
        let matrix = Mat4::from_cols_array(&r.floats::<16>()?);
        if model as usize >= models.len() {
            continue;
        }
        let clip = if clip >= 0 && clips.get(clip as usize).is_some_and(|c| c.model == model) { clip } else { -1 };
        instances.push(Instance {
            model,
            clip,
            phase,
            flags,
            tint,
            matrix,
        });
    }
    Ok(AnimData {
        textures,
        materials,
        models,
        clips,
        instances,
    })
}

fn anim_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("HORIZON_DOTA_ANIM") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }
    let mut candidates = Vec::new();
    if let Ok(map) = std::env::var("HORIZON_DOTA_MAP") {
        if let Some(dir) = Path::new(&map).parent() {
            candidates.push(dir.join("anim.bin"));
        }
    }
    candidates.push(PathBuf::from(r"C:\Users\root\Desktop\rust\dota-map-data\anim.bin"));
    candidates.push(PathBuf::from("dota-map-data").join("anim.bin"));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("dota-map-data").join("anim.bin"));
            candidates.push(dir.join("..").join("dota-map-data").join("anim.bin"));
            candidates.push(dir.join("..").join("..").join("dota-map-data").join("anim.bin"));
        }
    }
    candidates.into_iter().find(|path| path.exists())
}

/// A decoded texture with its box-filtered mip chain.
struct Image {
    width: u32,
    height: u32,
    levels: u32,
    data: Vec<u8>,
}

fn load_rgba(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let file = File::open(path).ok()?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut bytes = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut bytes).ok()?;
    let channels = info.color_type.samples();
    let mut pixels = Vec::with_capacity(info.width as usize * info.height as usize * 4);
    for p in bytes[..info.buffer_size()].chunks_exact(channels) {
        pixels.extend(match info.color_type {
            png::ColorType::Rgba => [p[0], p[1], p[2], p[3]],
            png::ColorType::Rgb => [p[0], p[1], p[2], 255],
            png::ColorType::Grayscale => [p[0], p[0], p[0], 255],
            png::ColorType::GrayscaleAlpha => [p[0], p[0], p[0], p[1]],
            _ => return None,
        });
    }
    Some((info.width, info.height, pixels))
}

fn mip_chain(width: u32, height: u32, pixels: Vec<u8>) -> Image {
    let mut levels = 1;
    let mut data = pixels.clone();
    let (mut w, mut h) = (width as usize, height as usize);
    let mut level = pixels;
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
    Image {
        width,
        height,
        levels,
        data,
    }
}

enum Msg {
    Data(Box<AnimData>),
    Texture(usize, Option<Image>),
}

enum Stage {
    Idle,
    Loading {
        rx: Receiver<Msg>,
        data: Option<Box<AnimData>>,
        views: Vec<Option<wgpu::TextureView>>,
        received: usize,
    },
    Done,
}

fn start_loading() -> Option<Receiver<Msg>> {
    let path = anim_path()?;
    let (tx, rx) = sync_channel(2);
    let spawned = std::thread::Builder::new().name("dota-anim-load".into()).spawn(move || {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("Dota anim: cannot read {}: {error}", path.display());
                return;
            }
        };
        let root = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let data = match parse_anim(&root, &bytes) {
            Ok(data) => data,
            Err(error) => {
                log::warn!("Dota anim: {error}");
                return;
            }
        };
        drop(bytes);
        let textures = data.textures.clone();
        log::info!(
            "Dota anim: {} models, {} clips, {} instances, {} textures",
            data.models.len(),
            data.clips.len(),
            data.instances.len(),
            textures.len()
        );
        if tx.send(Msg::Data(Box::new(data))).is_err() {
            return;
        }
        for (i, path) in textures.iter().enumerate() {
            let image = load_rgba(path).map(|(w, h, pixels)| mip_chain(w, h, pixels));
            if image.is_none() {
                log::warn!("Dota anim: missing texture {}", path.display());
            }
            if tx.send(Msg::Texture(i, image)).is_err() {
                return;
            }
        }
    });
    spawned.ok()?;
    Some(rx)
}

fn image_texture(device: &wgpu::Device, queue: &wgpu::Queue, image: &Image) -> wgpu::TextureView {
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("Dota anim texture"),
            size: wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: image.levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &image.data,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn solid_texture(device: &wgpu::Device, queue: &wgpu::Queue, rgba: [u8; 4]) -> wgpu::TextureView {
    image_texture(
        device,
        queue,
        &Image {
            width: 1,
            height: 1,
            levels: 1,
            data: rgba.to_vec(),
        },
    )
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuGlobals {
    view_proj: [[f32; 4]; 4],
    shadow_view_proj: [[f32; 4]; 4],
    camera: [f32; 4],
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
    light_grid: [f32; 4],
    light_grid2: [f32; 4],
    flags: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuMaterial {
    kind: u32,
    features: u32,
    mask_scale: f32,
    pad0: f32,
    scales: [f32; 4],
    offset: [f32; 4],
    tint: [f32; 4],
    params: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuInstance {
    matrix: [[f32; 4]; 4],
    tint_fade: [f32; 4],
    palette: [u32; 4],
}

struct DrawMesh {
    material: usize,
    first_index: u32,
    index_count: u32,
    base_vertex: i32,
}

struct DrawModel {
    meshes: Vec<DrawMesh>,
    instances: std::ops::Range<u32>,
}

/// One palette the frame samples: a model playing a clip from a phase.
struct PaletteKey {
    model: usize,
    clip: Option<usize>,
    phase: f32,
}

struct Frame {
    fades: Vec<f32>,
    last: Option<Instant>,
    uploaded: Vec<GpuInstance>,
    scratch: Vec<Mat4>,
    palettes: Vec<[[f32; 4]; 4]>,
}

struct Gpu {
    data: Box<AnimData>,
    pipelines: [wgpu::RenderPipeline; 3],
    material_groups: Vec<wgpu::BindGroup>,
    material_pass: Vec<usize>,
    _views: Vec<wgpu::TextureView>,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    instance_buf: wgpu::Buffer,
    globals: wgpu::Buffer,
    palette_buf: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    fallback_shadow: wgpu::BindGroup,
    models: Vec<DrawModel>,
    keys: Vec<PaletteKey>,
    /// Instances in draw order (sorted by model): index into data.instances.
    order: Vec<usize>,
    /// Palette entries the placed instances use; creatures' come after.
    static_palette: u32,
    model_by_name: std::collections::HashMap<String, usize>,
    clip_by_name: std::collections::HashMap<(usize, String), usize>,
    frame: Mutex<Frame>,
    started: Instant,
}

static STAGE: Mutex<Stage> = Mutex::new(Stage::Idle);
static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

/// Draws the animated map models inside the world pass, after the map's
/// opaque batches. Leaves group 1 bound to `light.shadow_group` when given.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pass: &mut wgpu::RenderPass<'_>,
    format: wgpu::TextureFormat,
    camera: [f32; 3],
    view_proj: Mat4,
    day: f32,
    light: &Lighting<'_>,
) {
    if let Some(gpu) = GPU.get() {
        if let Some(gpu) = gpu {
            gpu.draw(queue, pass, camera, view_proj, day, light);
        }
        return;
    }
    let Ok(mut stage) = STAGE.lock() else {
        return;
    };
    if matches!(*stage, Stage::Idle) {
        *stage = match start_loading() {
            Some(rx) => Stage::Loading {
                rx,
                data: None,
                views: Vec::new(),
                received: 0,
            },
            None => {
                let _ = GPU.set(None);
                Stage::Done
            }
        };
    }
    let Stage::Loading {
        rx,
        data,
        views,
        received,
    } = &mut *stage
    else {
        return;
    };
    let mut uploaded = 0;
    let mut gone = false;
    while uploaded < TEXTURES_PER_FRAME {
        match rx.try_recv() {
            Ok(Msg::Data(d)) => {
                *views = (0..d.textures.len()).map(|_| None).collect();
                *data = Some(d);
            }
            Ok(Msg::Texture(i, image)) => {
                if let (Some(slot), Some(image)) = (views.get_mut(i), image) {
                    *slot = Some(image_texture(device, queue, &image));
                }
                *received += 1;
                uploaded += 1;
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                gone = true;
                break;
            }
        }
    }
    let complete = data.as_ref().is_some_and(|d| *received >= d.textures.len());
    if !complete && !gone {
        return;
    }
    let gpu = data.take().filter(|_| complete).and_then(|d| Gpu::new(device, queue, format, d, std::mem::take(views)));
    if gpu.is_none() {
        log::warn!("Dota anim: not drawing animated models");
    }
    let _ = GPU.set(gpu);
    *stage = Stage::Done;
}

fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Unique palettes and, per instance, which one it uses.
fn palette_keys(data: &AnimData) -> (Vec<PaletteKey>, Vec<usize>) {
    let mut keys: Vec<PaletteKey> = Vec::new();
    let mut of_instance = Vec::with_capacity(data.instances.len());
    for inst in &data.instances {
        let clip = (inst.clip >= 0).then_some(inst.clip as usize);
        let model = inst.model as usize;
        let at = keys
            .iter()
            .position(|k| k.model == model && k.clip == clip && k.phase.to_bits() == inst.phase.to_bits())
            .unwrap_or_else(|| {
                keys.push(PaletteKey {
                    model,
                    clip,
                    phase: inst.phase,
                });
                keys.len() - 1
            });
        of_instance.push(at);
    }
    (keys, of_instance)
}

impl Gpu {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        data: Box<AnimData>,
        loaded: Vec<Option<wgpu::TextureView>>,
    ) -> Option<Self> {
        if data.instances.is_empty() {
            return None;
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Dota anim"),
            source: wgpu::ShaderSource::Wgsl(include_str!("dota_anim.wgsl").into()),
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Dota anim material"),
            entries: &[
                uniform_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex_entry(2),
                tex_entry(3),
                tex_entry(4),
                tex_entry(5),
                tex_entry(6),
                tex_entry(7),
            ],
        });
        // Exactly dota_map's shadow layout, so its bind group fits.
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
                storage_entry(2, wgpu::ShaderStages::FRAGMENT),
                storage_entry(3, wgpu::ShaderStages::FRAGMENT),
                // dota_map's cloud shadow texture, unused here.
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // dota_map's ambient occlusion volume, its sampler and its extent.
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
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Dota anim globals"),
            entries: &[uniform_entry(0), storage_entry(1, wgpu::ShaderStages::VERTEX)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Dota anim layout"),
            bind_group_layouts: &[Some(&material_layout), Some(&shadow_layout), Some(&globals_layout)],
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x3,
            1 => Float32x3,
            2 => Float32x4,
            3 => Float32x2,
            4 => Uint16x4,
            5 => Float32x4
        ];
        let instance_attributes = wgpu::vertex_attr_array![
            6 => Float32x4,
            7 => Float32x4,
            8 => Float32x4,
            9 => Float32x4,
            10 => Float32x4,
            11 => Uint32
        ];
        let buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<AnimVert>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<GpuInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance_attributes,
            }),
        ];
        let pipeline_for = |blend: Option<wgpu::BlendState>, depth_write: bool, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
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
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
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
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let pipelines = [
            pipeline_for(None, true, "Dota anim"),
            pipeline_for(Some(wgpu::BlendState::ALPHA_BLENDING), false, "Dota anim blend"),
            pipeline_for(Some(additive), false, "Dota anim additive"),
        ];
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Dota anim sampler"),
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
        let views: Vec<wgpu::TextureView> = loaded
            .into_iter()
            .map(|v| v.unwrap_or_else(|| solid_texture(device, queue, [180, 170, 150, 255])))
            .collect();
        let mut material_groups = Vec::with_capacity(data.materials.len());
        let mut material_pass = Vec::with_capacity(data.materials.len());
        for mat in &data.materials {
            let gpu_mat = GpuMaterial {
                kind: mat.kind,
                features: mat.features,
                mask_scale: mat.mask_scale,
                pad0: 0.0,
                scales: mat.scales,
                offset: mat.offset,
                tint: mat.tint,
                params: [
                    [mat.params[0], mat.params[1], mat.params[2], mat.params[3]],
                    [mat.params[4], mat.params[5], mat.params[6], mat.params[7]],
                    [mat.params[8], mat.params[9], mat.params[10], mat.params[11]],
                    [mat.params[12], mat.params[13], mat.params[14], mat.params[15]],
                ],
            };
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Dota anim material"),
                contents: bytemuck::bytes_of(&gpu_mat),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let view = |i: usize| {
                let default = match i {
                    3 => &flat,
                    4 | 5 => &black,
                    _ => &white,
                };
                let slot = mat.slots[i];
                if slot >= 0 { views.get(slot as usize).unwrap_or(default) } else { default }
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
            material_groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Dota anim material"),
                layout: &material_layout,
                entries: &entries,
            }));
            material_pass.push(if mat.kind == KIND_SOLID { 0 } else { mat.pass() });
        }

        let mut all_verts: Vec<AnimVert> = Vec::new();
        let mut all_indices: Vec<u32> = Vec::new();
        let mut order: Vec<usize> = (0..data.instances.len()).collect();
        order.sort_by_key(|&i| data.instances[i].model);
        let mut models = Vec::with_capacity(data.models.len());
        for (mi, model) in data.models.iter().enumerate() {
            let start = order.partition_point(|&i| (data.instances[i].model as usize) < mi) as u32;
            let end = order.partition_point(|&i| (data.instances[i].model as usize) <= mi) as u32;
            let mut meshes = Vec::with_capacity(model.meshes.len());
            for mesh in &model.meshes {
                meshes.push(DrawMesh {
                    material: mesh.material as usize,
                    first_index: all_indices.len() as u32,
                    index_count: mesh.indices.len() as u32,
                    base_vertex: all_verts.len() as i32,
                });
                all_verts.extend_from_slice(&mesh.verts);
                all_indices.extend_from_slice(&mesh.indices);
            }
            models.push(DrawModel {
                meshes,
                instances: start..end,
            });
        }
        if all_indices.is_empty() {
            return None;
        }
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dota anim vertices"),
            contents: bytemuck::cast_slice(&all_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dota anim indices"),
            contents: bytemuck::cast_slice(&all_indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        drop(all_verts);
        drop(all_indices);

        let (keys, key_of) = palette_keys(&data);
        let mut key_base = Vec::with_capacity(keys.len());
        let mut total = 0u32;
        for key in &keys {
            key_base.push(total);
            total += data.models[key.model].bones.len().max(1) as u32;
        }
        let uploaded: Vec<GpuInstance> = order
            .iter()
            .map(|&i| {
                let inst = &data.instances[i];
                GpuInstance {
                    matrix: inst.matrix.to_cols_array_2d(),
                    tint_fade: [inst.tint[0], inst.tint[1], inst.tint[2], 0.0],
                    palette: [key_base[key_of[i]], 0, 0, 0],
                }
            })
            .collect();
        let mut padded = uploaded.clone();
        padded.resize(uploaded.len() + CREATURES_MAX, bytemuck::Zeroable::zeroed());
        let instance_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dota anim instances"),
            contents: bytemuck::cast_slice(&padded),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let palette_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dota anim palettes"),
            size: (total as u64 + (CREATURES_MAX * CREATURE_BONES) as u64).max(1) * 64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dota anim globals"),
            size: std::mem::size_of::<GpuGlobals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Dota anim globals"),
            layout: &globals_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: palette_buf.as_entire_binding(),
                },
            ],
        });
        let fallback_shadow = Self::fallback_shadow(device, queue, &shadow_layout);
        log::info!(
            "Dota anim: uploaded {} materials, {} palettes ({} bones)",
            material_groups.len(),
            keys.len(),
            total
        );
        let count = data.instances.len();
        let model_by_name = data.models.iter().enumerate().map(|(i, m)| (m.name.clone(), i)).collect();
        let clip_by_name = data
            .clips
            .iter()
            .enumerate()
            .map(|(i, c)| ((c.model as usize, c.name.rsplit('_').next().unwrap_or("").to_lowercase()), i))
            .collect();
        Some(Self {
            static_palette: total,
            model_by_name,
            clip_by_name,
            data,
            pipelines,
            material_groups,
            material_pass,
            _views: views,
            vertices,
            indices,
            instance_buf,
            globals,
            palette_buf,
            globals_group,
            fallback_shadow,
            models,
            keys,
            order,
            frame: Mutex::new(Frame {
                fades: vec![-1.0; count],
                last: None,
                uploaded,
                scratch: Vec::new(),
                palettes: Vec::with_capacity(total as usize),
            }),
            started: Instant::now(),
        })
    }

    /// Group 1 when dota_map gives none: a zeroed 1x1 shadow map (never
    /// sampled, flags.x is 0) and no point lights.
    fn fallback_shadow(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout) -> wgpu::BindGroup {
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Dota anim no shadow"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Dota anim no shadow"),
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let lights = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dota anim no lights"),
            contents: bytemuck::cast_slice(&[[0.0f32; 4]; 2]),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let clouds = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Dota anim no clouds"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let cells = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dota anim no light cells"),
            contents: bytemuck::cast_slice(&[0u32; 4]),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let no_volume = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("Dota anim no ambient occlusion"),
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
            .create_view(&wgpu::TextureViewDescriptor::default());
        let volume_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Dota anim no ambient occlusion"),
            ..Default::default()
        });
        let no_info = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dota anim no ambient occlusion"),
            contents: bytemuck::cast_slice(&[[0.0f32; 4]; 2]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Dota anim no shadow"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: lights.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: cells.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&clouds),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&no_volume),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&volume_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: no_info.as_entire_binding(),
                },
            ],
        })
    }

    fn draw(
        &self,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        camera: [f32; 3],
        view_proj: Mat4,
        day: f32,
        light: &Lighting<'_>,
    ) {
        let Ok(mut frame) = self.frame.lock() else {
            return;
        };
        let frame = &mut *frame;
        let now = Instant::now();
        let dt = frame.last.map(|l| now.duration_since(l).as_secs_f32()).unwrap_or(0.0).min(0.25);
        frame.last = Some(now);
        let time = now.duration_since(self.started).as_secs_f32();
        let is_day = day >= 0.5;
        let mut changed = false;
        let mut any = false;
        for (slot, &i) in self.order.iter().enumerate() {
            let target = shown(self.data.instances[i].flags, is_day);
            let old = frame.fades[slot];
            let fade = if old < 0.0 { if target { 1.0 } else { 0.0 } } else { step_fade(old, target, dt) };
            if fade != old {
                frame.fades[slot] = fade;
                frame.uploaded[slot].tint_fade[3] = fade;
                changed = true;
            }
            any |= fade > 0.0;
        }
        if changed {
            queue.write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(&frame.uploaded));
        }
        let mut creatures: Vec<(usize, Creature)> = CREATURES
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|c| Some((*self.model_by_name.get(&c.model)?, c)))
            .take(CREATURES_MAX)
            .collect();
        creatures.sort_by_key(|(m, _)| *m);
        if !any && creatures.is_empty() {
            return;
        }
        frame.palettes.clear();
        for key in &self.keys {
            let model = &self.data.models[key.model];
            let clip = key.clip.and_then(|c| self.data.clips.get(c));
            model.palette(clip, time + key.phase, &mut frame.scratch, &mut frame.palettes);
            if model.bones.is_empty() {
                frame.palettes.push(Mat4::IDENTITY.to_cols_array_2d());
            }
        }
        // Creatures: one palette and one instance each, after the placed ones.
        let mut dynamic: Vec<GpuInstance> = Vec::with_capacity(creatures.len());
        let mut ranges: Vec<std::ops::Range<u32>> = vec![0..0; self.data.models.len()];
        let static_count = self.order.len() as u32;
        let mut palette_next = self.static_palette;
        for (k, (mi, c)) in creatures.iter().enumerate() {
            let model = &self.data.models[*mi];
            let clip = self.clip_by_name.get(&(*mi, c.clip.clone())).and_then(|&ci| self.data.clips.get(ci));
            let mut one = Vec::new();
            model.palette(clip, c.time, &mut frame.scratch, &mut one);
            if one.is_empty() {
                one.push(Mat4::IDENTITY.to_cols_array_2d());
            }
            one.truncate(CREATURE_BONES);
            let base = palette_next;
            palette_next += CREATURE_BONES as u32;
            frame.palettes.resize(base as usize, Mat4::IDENTITY.to_cols_array_2d());
            frame.palettes.extend_from_slice(&one);
            dynamic.push(GpuInstance {
                matrix: c.matrix.to_cols_array_2d(),
                tint_fade: [c.tint[0], c.tint[1], c.tint[2], c.fade],
                palette: [base, 0, 0, 0],
            });
            let r = &mut ranges[*mi];
            if r.is_empty() {
                *r = static_count + k as u32..static_count + k as u32 + 1;
            } else {
                r.end = static_count + k as u32 + 1;
            }
        }
        if !dynamic.is_empty() {
            queue.write_buffer(
                &self.instance_buf,
                static_count as u64 * std::mem::size_of::<GpuInstance>() as u64,
                bytemuck::cast_slice(&dynamic),
            );
        }
        queue.write_buffer(&self.palette_buf, 0, bytemuck::cast_slice(&frame.palettes));
        let globals = GpuGlobals {
            view_proj: view_proj.to_cols_array_2d(),
            shadow_view_proj: light.shadow_view_proj.to_cols_array_2d(),
            camera: [camera[0], camera[1], camera[2], time],
            sun_dir: light.sun_dir,
            sun_color: light.sun_color,
            ambient_dir: light.ambient_dir,
            ambient_color: light.ambient_color,
            sky_vertical: light.sky_vertical,
            sky_constant: light.sky_constant,
            specular_dir: light.specular_dir,
            specular_albedo: light.specular_albedo,
            specular_lit: light.specular_lit,
            fog_color: light.fog_color,
            light_grid: if light.shadow_group.is_some() { light.light_grid } else { [0.0; 4] },
            light_grid2: light.light_grid2,
            flags: [if light.shadow_group.is_some() { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        pass.set_bind_group(1, light.shadow_group.unwrap_or(&self.fallback_shadow), &[]);
        pass.set_bind_group(2, &self.globals_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_vertex_buffer(1, self.instance_buf.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        for (order, pipeline) in self.pipelines.iter().enumerate() {
            let mut set = false;
            for (mi, model) in self.models.iter().enumerate() {
                let shown = if any { model.instances.clone() } else { 0..0 };
                if shown.is_empty() && ranges[mi].is_empty() {
                    continue;
                }
                for mesh in &model.meshes {
                    if self.material_pass[mesh.material] != order {
                        continue;
                    }
                    if !set {
                        pass.set_pipeline(pipeline);
                        set = true;
                    }
                    pass.set_bind_group(0, &self.material_groups[mesh.material], &[]);
                    for range in [shown.clone(), ranges[mi].clone()] {
                        if !range.is_empty() {
                            pass.draw_indexed(
                                mesh.first_index..mesh.first_index + mesh.index_count,
                                mesh.base_vertex,
                                range,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trs(t: [f32; 3]) -> Trs {
        Trs {
            t: Vec3::from(t),
            r: Quat::IDENTITY,
            s: Vec3::ONE,
        }
    }

    fn clip(frames: u32, fps: f32, tracks: Vec<(u32, u32)>, keys: Vec<Trs>) -> Clip {
        Clip {
            model: 0,
            name: "test".into(),
            fps,
            frames,
            tracks,
            keys,
        }
    }

    fn chain() -> Model {
        let parents = vec![-1, 0];
        Model {
            name: "chain".into(),
            order: node_order(&parents),
            parents,
            rest: vec![trs([0.0, 1.0, 0.0]), trs([0.0, 2.0, 0.0])],
            bones: vec![
                (0, Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0))),
                (1, Mat4::from_translation(Vec3::new(0.0, -3.0, 0.0))),
            ],
            meshes: Vec::new(),
        }
    }

    #[test]
    fn frames_interpolate_and_loop() {
        let c = clip(3, 2.0, vec![], vec![]);
        assert_eq!(c.duration(), 1.0);
        let (a, b, f) = c.frame_at(0.25);
        assert_eq!((a, b), (0, 1));
        assert!((f - 0.5).abs() < 1e-5);
        let (a, b, f) = c.frame_at(0.75);
        assert_eq!((a, b), (1, 2));
        assert!((f - 0.5).abs() < 1e-5);
        let (a, _, f) = c.frame_at(1.25);
        assert_eq!(a, 0);
        assert!((f - 0.5).abs() < 1e-5);
        let (a, _, _) = c.frame_at(-0.25);
        assert_eq!(a, 1);
        assert_eq!(clip(1, 30.0, vec![], vec![]).frame_at(5.0), (0, 0, 0.0));
    }

    #[test]
    fn tracks_pick_rest_constant_or_sampled() {
        let rest = trs([9.0, 9.0, 9.0]);
        let keys = vec![trs([1.0, 0.0, 0.0]), trs([0.0, 0.0, 0.0]), trs([2.0, 0.0, 0.0])];
        let c = clip(2, 1.0, vec![(0, 0), (0, 1), (1, 2)], keys);
        let frame = c.frame_at(0.5);
        assert_eq!(c.local(0, &rest, frame), rest);
        assert_eq!(c.local(1, &rest, frame).t, Vec3::new(1.0, 0.0, 0.0));
        assert!((c.local(2, &rest, frame).t.x - 1.0).abs() < 1e-5);
        assert_eq!(c.local(7, &rest, frame), rest);
    }

    #[test]
    fn rotation_blend_takes_the_short_way() {
        let a = Trs {
            r: Quat::from_rotation_y(0.0),
            ..trs([0.0; 3])
        };
        let b = Trs {
            r: -Quat::from_rotation_y(1.0),
            ..trs([0.0; 3])
        };
        let mid = a.lerp(&b, 0.5);
        let (axis, angle) = mid.r.to_axis_angle();
        assert!((angle - 0.5).abs() < 0.01, "angle {angle} axis {axis}");
        assert!((mid.r.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn rest_palette_is_identity() {
        let model = chain();
        let mut scratch = Vec::new();
        let mut out = Vec::new();
        model.palette(None, 0.0, &mut scratch, &mut out);
        assert_eq!(out.len(), 2);
        for m in &out {
            let m = Mat4::from_cols_array_2d(m);
            assert!(m.abs_diff_eq(Mat4::IDENTITY, 1e-5), "{m}");
        }
    }

    #[test]
    fn palette_follows_the_parent() {
        let model = chain();
        let keys = vec![trs([0.0, 1.0, 0.0]), trs([5.0, 1.0, 0.0])];
        let c = clip(2, 1.0, vec![(0, 2), (0, 0)], keys);
        let mut scratch = Vec::new();
        let mut out = Vec::new();
        model.palette(Some(&c), 0.5, &mut scratch, &mut out);
        for m in &out {
            let p = Mat4::from_cols_array_2d(m).transform_point3(Vec3::new(0.0, 2.0, 0.0));
            assert!((p - Vec3::new(2.5, 2.0, 0.0)).length() < 1e-4, "{p}");
        }
    }

    #[test]
    fn parents_are_ordered_first() {
        let parents = vec![2, -1, 1, 0];
        let order = node_order(&parents);
        let pos = |n: usize| order.iter().position(|&o| o == n).unwrap();
        assert_eq!(order.len(), 4);
        assert!(pos(1) < pos(2) && pos(2) < pos(0) && pos(0) < pos(3));
        assert_eq!(node_order(&[1, 0]).len(), 2);
    }

    #[test]
    fn day_and_night_visibility() {
        assert!(shown(0, true) && shown(0, false));
        assert!(shown(SHOW_DAY, true) && !shown(SHOW_DAY, false));
        assert!(!shown(SHOW_NIGHT, true) && shown(SHOW_NIGHT, false));
        assert_eq!(step_fade(0.0, true, FADE_SECONDS * 0.5), 0.5);
        assert_eq!(step_fade(0.2, false, 1.0), 0.0);
    }

    fn put_str(out: &mut Vec<u8>, s: &str) {
        out.extend((s.len() as u16).to_le_bytes());
        out.extend(s.as_bytes());
    }

    fn put_f(out: &mut Vec<u8>, v: &[f32]) {
        for x in v {
            out.extend(x.to_le_bytes());
        }
    }

    #[test]
    fn parses_a_small_file() {
        let mut b = b"DOTAANM1".to_vec();
        b.extend(1u32.to_le_bytes());
        put_str(&mut b, "anim/a.png");
        b.extend(1u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend(2u32.to_le_bytes());
        for s in [0i32, -1, -1, -1, -1, -1] {
            b.extend(s.to_le_bytes());
        }
        put_f(&mut b, &[1.0; 12]);
        put_f(&mut b, &[0.0; 16]);
        put_f(&mut b, &[1.0]);
        b.extend(1u32.to_le_bytes());
        put_str(&mut b, "m");
        b.extend(1u32.to_le_bytes());
        b.extend((-1i32).to_le_bytes());
        put_f(&mut b, &[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]);
        b.extend(1u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        put_f(&mut b, &Mat4::IDENTITY.to_cols_array());
        b.extend(1u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend(3u32.to_le_bytes());
        b.extend(3u32.to_le_bytes());
        let vert = AnimVert {
            pos: [0.0; 3],
            nrm: [0.0, 1.0, 0.0],
            tan: [1.0, 0.0, 0.0, 1.0],
            uv: [0.0; 2],
            joints: [0, 7, 0, 0],
            weights: [1.0, 0.0, 0.0, 0.0],
        };
        for _ in 0..3 {
            b.extend(bytemuck::bytes_of(&vert));
        }
        for i in 0u32..3 {
            b.extend(i.to_le_bytes());
        }
        b.extend(1u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        put_str(&mut b, "idle");
        put_f(&mut b, &[30.0]);
        b.extend(4u32.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        put_f(&mut b, &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]);
        b.extend(1u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend(0i32.to_le_bytes());
        put_f(&mut b, &[0.0]);
        b.extend(SHOW_DAY.to_le_bytes());
        put_f(&mut b, &[1.0, 0.5, 0.25]);
        put_f(&mut b, &Mat4::IDENTITY.to_cols_array());
        let data = parse_anim(Path::new("root"), &b).expect("parse");
        assert_eq!(data.textures, vec![Path::new("root").join("anim/a.png")]);
        assert_eq!(data.materials[0].features, 2);
        assert_eq!(data.models[0].meshes[0].verts[0].joints[1], 0, "out of range joint clamped");
        assert_eq!(data.clips[0].frames, 4);
        assert_eq!(data.clips[0].tracks, vec![(0, 1)]);
        assert_eq!(data.instances[0].clip, 0);
        assert_eq!(data.instances[0].tint, [1.0, 0.5, 0.25]);
        let (keys, of) = palette_keys(&data);
        assert_eq!((keys.len(), of), (1, vec![0]));
        assert!(parse_anim(Path::new("root"), &b[..b.len() - 4]).is_err());
    }

    #[test]
    fn shader_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("dota_anim.wgsl")).expect("parse");
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .expect("validate");
        assert_eq!(std::mem::size_of::<AnimVert>(), 72);
        assert_eq!(std::mem::size_of::<GpuInstance>(), 96);
    }

    #[test]
    #[ignore = "needs the local dota-map-data/anim.bin"]
    fn real_anim_bin_rest_poses_bind() {
        let path = anim_path().expect("anim.bin");
        let bytes = std::fs::read(&path).unwrap();
        let data = parse_anim(path.parent().unwrap(), &bytes).unwrap();
        let mut scratch = Vec::new();
        for model in &data.models {
            let mut out = Vec::new();
            model.palette(None, 0.0, &mut scratch, &mut out);
            let worst = out
                .iter()
                .map(|m| {
                    let d = Mat4::from_cols_array_2d(m) - Mat4::IDENTITY;
                    d.to_cols_array().iter().fold(0.0f32, |a, v| a.max(v.abs()))
                })
                .fold(0.0, f32::max);
            assert!(worst < 1e-3, "{} rest palette off by {worst}", model.name);
        }
        for c in &data.clips {
            let model = &data.models[c.model as usize];
            let mut out = Vec::new();
            model.palette(Some(c), 1.3, &mut scratch, &mut out);
            assert!(out.iter().flatten().flatten().all(|v| v.is_finite()), "{}", c.name);
        }
        let (keys, _) = palette_keys(&data);
        eprintln!("{} instances, {} palettes", data.instances.len(), keys.len());
    }
}
