use crate::font::Patch;
use crate::hud::{HudAtlas, HudBatch, Rect};
use ab_glyph::{Font as _, FontVec, PxScale, ScaleFont as _};
use std::{cell::RefCell, collections::HashMap};

const REGION: &str = "horizon/dota-text";
const FACES: [&str; 3] = ["radiance", "radiancem", "reaver"];
const WEIGHTS: [(u16, &str); 5] = [(300, "light"), (400, "regular"), (600, "semibold"), (700, "bold"), (900, "black")];

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Face {
    family: u8,
    weight: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    face: Face,
    glyph: u16,
    size: u16,
}

#[derive(Clone, Copy)]
struct Cached {
    rect: Option<Rect>,
    left: f32,
    top: f32,
}

struct Cache {
    fonts: HashMap<Face, FontVec>,
    root: Option<std::path::PathBuf>,
    entries: HashMap<Key, Cached>,
    shelf: (u32, u32, u32),
    pending: Vec<Patch>,
    atlas: usize,
}

thread_local! {
    static CACHE: RefCell<Option<Cache>> = const { RefCell::new(None) };
}

pub fn reset_atlas() {
    CACHE.with(|c| *c.borrow_mut() = None);
}

pub fn family_index(name: &str) -> Option<u8> {
    let lower = name.to_ascii_lowercase();
    FACES.iter().position(|f| *f == lower).map(|i| i as u8)
}

fn weight_index(weight: u16) -> u8 {
    WEIGHTS
        .iter()
        .enumerate()
        .min_by_key(|(_, (w, _))| (i32::from(*w) - i32::from(weight)).abs())
        .map_or(1, |(i, _)| i as u8)
}

fn with_cache<R>(atlas: &HudAtlas, f: impl FnOnce(&mut Cache) -> R) -> R {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let id = atlas as *const HudAtlas as usize;
        let cache = c.get_or_insert_with(|| Cache {
            fonts: HashMap::new(),
            root: horizon_dota::data::find_data_dir().map(|d| d.join("fonts")),
            entries: HashMap::new(),
            shelf: (0, 0, 0),
            pending: Vec::new(),
            atlas: id,
        });
        if cache.atlas != id {
            cache.atlas = id;
            cache.entries.clear();
            cache.shelf = (0, 0, 0);
            cache.pending.clear();
        }
        f(cache)
    })
}

impl Cache {
    fn face_for(&mut self, face: Face, ch: char) -> Face {
        let has = |cache: &mut Cache, f: Face| cache.font(f).is_some_and(|font| font.glyph_id(ch).0 != 0);
        if has(self, face) {
            return face;
        }
        let fallback = Face { family: 0, weight: face.weight };
        if face.family != 0 && has(self, fallback) {
            return fallback;
        }
        face
    }

    fn font(&mut self, face: Face) -> Option<&FontVec> {
        if !self.fonts.contains_key(&face) {
            let root = self.root.as_ref()?;
            let family = FACES[face.family as usize];
            let order: Vec<usize> = {
                let want = face.weight as usize;
                let mut o: Vec<usize> = (0..WEIGHTS.len()).collect();
                o.sort_by_key(|i| (*i as i32 - want as i32).abs());
                o
            };
            for i in order {
                let path = root.join(format!("{family}-{}.otf", WEIGHTS[i].1));
                if let Ok(bytes) = std::fs::read(&path)
                    && let Ok(font) = FontVec::try_from_vec(bytes)
                {
                    self.fonts.insert(face, font);
                    break;
                }
            }
        }
        self.fonts.get(&face)
    }

    fn place(&mut self, atlas: &HudAtlas, w: u32, h: u32) -> Option<(u32, u32, Rect)> {
        let region = atlas.sprites.get(REGION).copied()?;
        let (mut x, mut y, mut row) = self.shelf;
        if x + w + 1 > region.w {
            x = 0;
            y += row + 1;
            row = 0;
        }
        if y + h > region.h || w > region.w {
            self.entries.clear();
            self.shelf = (0, 0, 0);
            return None;
        }
        self.shelf = (x + w + 1, y, row.max(h));
        Some((x, y, region))
    }

    fn upload(&mut self, atlas: &HudAtlas, w: u32, h: u32, alpha: &[f32]) -> Option<Rect> {
        let (x, y, region) = self.place(atlas, w, h)?;
        let (ax, ay) = (
            (region.u0 * atlas.width as f32).round() as u32 + x,
            (region.v0 * atlas.height as f32).round() as u32 + y,
        );
        let rgba = alpha
            .iter()
            .flat_map(|a| [255, 255, 255, (a.clamp(0.0, 1.0) * 255.0).round() as u8])
            .collect();
        self.pending.push(Patch { x: ax, y: ay, width: w, height: h, rgba });
        let (aw, ah) = (atlas.width as f32, atlas.height as f32);
        Some(Rect {
            u0: ax as f32 / aw,
            v0: ay as f32 / ah,
            u1: (ax + w) as f32 / aw,
            v1: (ay + h) as f32 / ah,
            w,
            h,
        })
    }

    fn glyph(&mut self, atlas: &HudAtlas, face: Face, glyph: ab_glyph::GlyphId, px: f32) -> Cached {
        let size = (px * 4.0).round() as u16;
        let key = Key { face, glyph: glyph.0, size };
        if let Some(c) = self.entries.get(&key) {
            return *c;
        }
        let empty = Cached { rect: None, left: 0.0, top: 0.0 };
        let Some(font) = self.font(face) else { return empty };
        let upem = font.units_per_em().unwrap_or(1000.0);
        let scale = PxScale::from(f32::from(size) / 4.0 * font.height_unscaled() / upem);
        let outlined = font.outline_glyph(glyph.with_scale_and_position(scale, ab_glyph::point(0.0, 0.0)));
        let Some(outline) = outlined else {
            self.entries.insert(key, empty);
            return empty;
        };
        let b = outline.px_bounds();
        let (w, h) = (b.width().ceil() as u32 + 1, b.height().ceil() as u32 + 1);
        let mut alpha = vec![0.0; (w * h) as usize];
        outline.draw(|x, y, c| {
            if x < w && y < h {
                alpha[(y * w + x) as usize] = c.powf(0.8);
            }
        });
        let made = Cached {
            rect: self.upload(atlas, w, h, &alpha),
            left: b.min.x,
            top: b.min.y,
        };
        if made.rect.is_some() {
            self.entries.insert(key, made);
        }
        made
    }
}

fn scale_for(font: &FontVec, px: f32) -> PxScale {
    let upem = font.units_per_em().unwrap_or(1000.0);
    PxScale::from(px * font.height_unscaled() / upem)
}

pub fn measure(atlas: &HudAtlas, text: &str, family: &str, weight: u16, size_gui: f32, gui_scale: f32) -> f32 {
    let Some(family) = family_index(family) else { return text.chars().count() as f32 * size_gui * 0.5 };
    let face = Face { family, weight: weight_index(weight) };
    let px = (size_gui * gui_scale).max(1.0);
    with_cache(atlas, |cache| {
        let mut width = 0.0;
        let mut previous: Option<(Face, ab_glyph::GlyphId)> = None;
        for ch in text.chars() {
            let used = cache.face_for(face, ch);
            let Some(font) = cache.font(used) else { return text.chars().count() as f32 * size_gui * 0.5 };
            let scaled = font.as_scaled(scale_for(font, px));
            let id = font.glyph_id(ch);
            if let Some((pf, p)) = previous
                && pf == used
            {
                width += scaled.kern(p, id);
            }
            width += scaled.h_advance(id);
            previous = Some((used, id));
        }
        width / gui_scale
    })
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    atlas: &HudAtlas,
    batch: &mut HudBatch,
    text: &str,
    family: &str,
    weight: u16,
    size_gui: f32,
    line_height_gui: f32,
    x: f32,
    y: f32,
    color: u32,
    shadow: Option<u32>,
) {
    let Some(family) = family_index(family) else { return };
    let face = Face { family, weight: weight_index(weight) };
    let gui_scale = batch.scale;
    let px = (size_gui * gui_scale).max(1.0);
    let pending = with_cache(atlas, |cache| {
        let (ascent, descent) = {
            let Some(font) = cache.font(face) else { return Vec::new() };
            let scaled = font.as_scaled(scale_for(font, px));
            (scaled.ascent(), scaled.descent())
        };
        let line = line_height_gui * gui_scale;
        let baseline_y = (y * gui_scale + (line - (ascent - descent)) / 2.0 + ascent).round();
        let mut pen = (x * gui_scale).round();
        let mut previous: Option<(Face, ab_glyph::GlyphId)> = None;
        let mut quads: Vec<(Rect, f32, f32)> = Vec::new();
        for ch in text.chars() {
            let used = cache.face_for(face, ch);
            let Some(font) = cache.font(used) else { continue };
            let id = font.glyph_id(ch);
            let scaled = font.as_scaled(scale_for(font, px));
            let kern = match previous {
                Some((pf, p)) if pf == used => scaled.kern(p, id),
                _ => 0.0,
            };
            let advance = scaled.h_advance(id);
            pen += kern;
            let g = cache.glyph(atlas, used, id, px);
            if let Some(rect) = g.rect {
                quads.push((rect, (pen + g.left).round(), (baseline_y + g.top).round()));
            }
            pen += advance;
            previous = Some((used, id));
        }
        for (rect, qx, qy) in quads {
            let (gx, gy) = (qx / gui_scale, qy / gui_scale);
            let (gw, gh) = (rect.w as f32 / gui_scale, rect.h as f32 / gui_scale);
            if let Some(shade) = shadow {
                let off = 1.0 / gui_scale;
                batch.rect_uv(gx + off, gy + off, gw, gh, rect, shade);
            }
            batch.rect_uv(gx, gy, gw, gh, rect, color);
        }
        std::mem::take(&mut cache.pending)
    });
    batch.patches.extend(pending);
}
