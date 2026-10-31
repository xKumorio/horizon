mod css;
mod layout;
mod paint;
mod tree;
mod value;
mod xml;

pub use layout::Ctx;
pub use tree::{Library, Panel, Rect};

use std::collections::HashMap;
use std::path::Path;

pub const REFERENCE_HEIGHT: f32 = 1080.0;

pub fn image_key(url: &str) -> Option<String> {
    let rest = url.strip_prefix("s2r://panorama/images/").or_else(|| url.strip_prefix("file://{images}/"))?;
    let rest = rest.split('?').next().unwrap_or(rest);
    let rest = rest.trim_end_matches(".vtex").trim_end_matches(".png").trim_end_matches(".vsvg");
    Some(rest.to_string())
}

pub struct Doc {
    pub root: Panel,
    pub lib: Library,
    pub strings: HashMap<String, String>,
    pub paint: icraft_ui::Paint,
    pub trace: Vec<String>,
}

pub type Measure<'a> = &'a dyn Fn(&str, &icraft_ui::Font) -> f32;
pub type Resolve<'a> = &'a dyn Fn(&str) -> Option<(String, f32, f32)>;

fn set_progress(p: &mut Panel) {
    if p.tag == "ProgressBar" {
        let v = p.value.clamp(0.0, 1.0);
        for c in &mut p.children {
            if c.has_class("ProgressBarLeft") {
                c.style.insert("width".into(), format!("{}%", v * 100.0));
                c.style.insert("height".into(), "100%".into());
            } else if c.has_class("ProgressBarRight") {
                c.style.insert("width".into(), format!("{}%", (1.0 - v) * 100.0));
            }
        }
    }
    for c in &mut p.children {
        set_progress(c);
    }
}

impl Doc {
    pub fn open(panorama_dir: &Path, layout: &str) -> Option<Doc> {
        let mut lib = Library::open(panorama_dir);
        let mut root = lib.load(layout)?;
        if root.tag == "DOTAHud" {
            for c in ["ShowLowerHUD", "ShowMinimap", "IsInGame", "Reborn", "ValidUnit", "IsHero", "HeroUnit", "ShowMana"] {
                root.set_class(c, true);
            }
        }
        Some(Doc { root, lib, strings: HashMap::new(), paint: icraft_ui::Paint::default(), trace: Vec::new() })
    }

    pub fn keep_only(&mut self, ids: &[&str]) {
        fn mark(p: &mut Panel, ids: &[&str]) -> bool {
            if ids.contains(&p.id.as_str()) {
                return true;
            }
            let mut any = false;
            for c in &mut p.children {
                any |= mark(c, ids);
            }
            if !any {
                p.hidden = true;
            }
            any
        }
        mark(&mut self.root, ids);
    }

    pub fn hide(&mut self, ids: &[&str]) {
        self.root.each_mut(&mut |p| {
            if ids.contains(&p.id.as_str()) {
                p.hidden = true;
            }
        });
    }

    pub fn fill_slots(&mut self, container: &str, count: usize, component: &str) {
        let inherited = self.root.find(container).map(|p| p.sheets.clone()).unwrap_or_default();
        let slots: Vec<Panel> = (0..count).filter_map(|_| self.lib.component(component, &inherited)).collect();
        if let Some(p) = self.root.find_mut(container) {
            p.children = slots;
        }
    }

    pub fn panel(&mut self, id: &str) -> Option<&mut Panel> {
        self.root.find_mut(id)
    }

    pub fn set_text(&mut self, id: &str, text: &str) {
        if let Some(p) = self.root.find_mut(id) {
            p.text = text.to_string();
        }
    }

    pub fn set_value(&mut self, id: &str, v: f32) {
        if let Some(p) = self.root.find_mut(id) {
            p.value = v;
        }
    }

    pub fn show(&mut self, id: &str, on: bool) {
        if let Some(p) = self.root.find_mut(id) {
            p.hidden = !on;
        }
    }

    pub fn class(&mut self, id: &str, class: &str, on: bool) {
        if let Some(p) = self.root.find_mut(id) {
            p.set_class(class, on);
        }
    }

    pub fn render(&mut self, width: f32, height: f32, measure: Measure, resolve: Resolve) -> icraft_ui::Paint {
        let k = height / REFERENCE_HEIGHT;
        let (w, h) = (width / k, REFERENCE_HEIGHT);
        tree::apply_styles(&mut self.root, &self.lib.sheet);
        set_progress(&mut self.root);
        let strings = &self.strings;
        let text = |p: &Panel| -> String {
            let raw = p.text.as_str();
            let mut out = if let Some(key) = raw.strip_prefix('#') { strings.get(key).cloned().unwrap_or_default() } else { raw.to_string() };
            while let Some(a) = out.find('{') {
                match out[a..].find('}') {
                    Some(b) => out.replace_range(a..a + b + 1, ""),
                    None => break,
                }
            }
            out
        };
        let image_size = |src: &str| resolve(src).map(|(_, w, h)| (w, h));
        let ctx = Ctx { measure, image_size: &image_size, text: &text };
        layout::arrange(&mut self.root, 0.0, 0.0, w, h, &ctx);
        let mut out = paint::Out { k, resolve, paint: icraft_ui::Paint::default(), current: String::new(), trace: Vec::new() };
        paint::paint(&self.root, &ctx, &mut out, 1.0);
        self.trace = std::mem::take(&mut out.trace);
        out.paint
    }
}
