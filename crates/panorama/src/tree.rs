use crate::css::{parse_decls, Compound, Rule, Selector, Sheet};
use crate::xml::{parse, Xml};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub type Style = HashMap<String, String>;

#[derive(Clone, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Panel {
    pub tag: String,
    pub id: String,
    pub classes: Vec<String>,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub image: String,
    pub value: f32,
    pub hidden: bool,
    pub click: String,
    pub style: Style,
    pub forced: Style,
    pub sheets: Vec<u32>,
    pub children: Vec<Panel>,
    pub rect: Rect,
}

impl Panel {
    pub fn new(tag: &str, classes: &[&str]) -> Panel {
        Panel { tag: tag.to_string(), classes: classes.iter().map(|c| c.to_string()).collect(), ..Panel::default() }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn style_force(&mut self, key: &str, value: &str) {
        self.forced.insert(key.to_string(), value.to_string());
    }

    pub fn has_class(&self, c: &str) -> bool {
        self.classes.iter().any(|x| x == c)
    }

    pub fn set_class(&mut self, c: &str, on: bool) {
        let has = self.has_class(c);
        if on && !has {
            self.classes.push(c.to_string());
        } else if !on && has {
            self.classes.retain(|x| x != c);
        }
    }

    pub fn find(&self, id: &str) -> Option<&Panel> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(id))
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Panel> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter_mut().find_map(|c| c.find_mut(id))
    }

    pub fn each_mut(&mut self, f: &mut dyn FnMut(&mut Panel)) {
        f(self);
        for c in &mut self.children {
            c.each_mut(f);
        }
    }

    pub fn count(&self) -> usize {
        1 + self.children.iter().map(Panel::count).sum::<usize>()
    }
}

pub struct Library {
    pub dir: PathBuf,
    pub sheet: Sheet,
    loaded: HashMap<String, u32>,
    components: HashMap<String, (Xml, Vec<String>)>,
    snippets: HashMap<String, (Xml, Vec<String>)>,
}

fn css_includes(root: &Xml) -> Vec<String> {
    let mut out = Vec::new();
    for c in &root.children {
        if c.tag == "styles" {
            for i in &c.children {
                if let Some(src) = i.attr("src") {
                    out.push(src.to_string());
                }
            }
        }
    }
    out
}

fn main_element(root: &Xml) -> Option<&Xml> {
    root.children.iter().find(|c| !matches!(c.tag.as_str(), "styles" | "scripts" | "snippets" | "include"))
}

fn walk_xml(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for e in read.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_xml(&p, out);
        } else if p.extension().is_some_and(|x| x == "xml") {
            out.push(p);
        }
    }
}

impl Library {
    pub fn open(panorama_dir: &Path) -> Library {
        let mut lib = Library {
            dir: panorama_dir.to_path_buf(),
            sheet: Sheet::default(),
            loaded: HashMap::new(),
            components: HashMap::new(),
            snippets: HashMap::new(),
        };
        let mut files = Vec::new();
        walk_xml(&panorama_dir.join("layout").join("hud"), &mut files);
        files.sort();
        for f in files {
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let Some(root) = parse(&text) else { continue };
            let styles = css_includes(&root);
            for sn in root.children.iter().filter(|c| c.tag == "snippets").flat_map(|c| c.children.iter()) {
                if let (Some(name), Some(body)) = (sn.attr("name"), sn.children.first()) {
                    lib.snippets.entry(name.to_string()).or_insert((body.clone(), styles.clone()));
                }
            }
            for sn in root.children.iter().filter(|c| c.tag == "snippet") {
                if let (Some(name), Some(body)) = (sn.attr("name"), sn.children.first()) {
                    lib.snippets.entry(name.to_string()).or_insert((body.clone(), styles.clone()));
                }
            }
            if let Some(main) = main_element(&root)
                && !matches!(main.tag.as_str(), "Panel" | "Label" | "Image" | "Button")
            {
                lib.components.entry(main.tag.clone()).or_insert((main.clone(), styles));
            }
        }
        lib
    }

    pub fn load_css(&mut self, src: &str) -> u32 {
        let rel = src.trim_start_matches("s2r://panorama/").trim_start_matches("file://{resources}/");
        let rel = rel.replace(".vcss", ".css");
        if let Some(id) = self.loaded.get(&rel) {
            return *id;
        }
        let id = self.loaded.len() as u32 + 1;
        self.loaded.insert(rel.clone(), id);
        if let Ok(text) = std::fs::read_to_string(self.dir.join(&rel)) {
            self.sheet.add(&text, id);
        }
        id
    }

    fn make(&mut self, x: &Xml, depth: u32, inherited: &[u32]) -> Panel {
        let mut p = Panel { tag: x.tag.clone(), attrs: x.attrs.clone(), sheets: inherited.to_vec(), ..Panel::default() };
        p.id = x.attr("id").unwrap_or("").to_string();
        p.classes = x.attr("class").unwrap_or("").split_whitespace().map(str::to_string).collect();
        p.text = x.attr("text").unwrap_or("").to_string();
        p.image = x.attr("src").unwrap_or("").to_string();
        p.click = x.attr("onactivate").unwrap_or("").to_string();
        if let Some(inline) = x.attr("style") {
            for (k, v) in parse_decls(inline) {
                p.forced.insert(k, v);
            }
        }
        let own: Vec<Xml> = match self.components.get(&x.tag).cloned() {
            Some((main, styles)) if depth < 14 => {
                for s in &styles {
                    let id = self.load_css(s);
                    if !p.sheets.contains(&id) {
                        p.sheets.push(id);
                    }
                }
                for (k, v) in &main.attrs {
                    if p.attr(k).is_none() {
                        p.attrs.push((k.clone(), v.clone()));
                    }
                }
                main.children.clone()
            }
            _ => Vec::new(),
        };
        let sheets = p.sheets.clone();
        for c in own.iter().chain(x.children.iter()) {
            if matches!(c.tag.as_str(), "styles" | "scripts" | "snippets" | "snippet" | "include") {
                continue;
            }
            let child = self.make(c, depth + 1, &sheets);
            p.children.push(child);
        }
        p
    }

    pub fn load(&mut self, layout: &str) -> Option<Panel> {
        let text = std::fs::read_to_string(self.dir.join(layout)).ok()?;
        let root = parse(&text)?;
        let mut sheets = Vec::new();
        for s in css_includes(&root) {
            let id = self.load_css(&s);
            if !sheets.contains(&id) {
                sheets.push(id);
            }
        }
        let main = main_element(&root)?.clone();
        let saved = self.components.remove(&main.tag);
        let panel = self.make(&main, 0, &sheets);
        if let Some(entry) = saved {
            self.components.insert(main.tag.clone(), entry);
        }
        Some(panel)
    }

    pub fn snippet(&mut self, name: &str, inherited: &[u32]) -> Option<Panel> {
        let (body, styles) = self.snippets.get(name)?.clone();
        let mut sheets = inherited.to_vec();
        for s in &styles {
            let id = self.load_css(s);
            if !sheets.contains(&id) {
                sheets.push(id);
            }
        }
        Some(self.make(&body, 1, &sheets))
    }

    pub fn component(&mut self, tag: &str, inherited: &[u32]) -> Option<Panel> {
        let (mut main, _) = self.components.get(tag)?.clone();
        main.children.clear();
        Some(self.make(&main, 0, inherited))
    }
}

struct Info {
    tag: String,
    id: String,
    classes: Vec<String>,
}

fn compound_matches(c: &Compound, n: &Info) -> bool {
    c.tag.as_deref().is_none_or(|t| t == n.tag)
        && c.id.as_deref().is_none_or(|i| i == n.id)
        && c.classes.iter().all(|k| n.classes.iter().any(|x| x == k))
        && !c.not_classes.iter().any(|k| n.classes.iter().any(|x| x == k))
}

fn selector_matches(sel: &Selector, node: &Info, anc: &[Info]) -> bool {
    let n = sel.parts.len();
    if !compound_matches(&sel.parts[n - 1].1, node) {
        return false;
    }
    fn up(sel: &Selector, idx: usize, anc: &[Info]) -> bool {
        if idx == 0 {
            return true;
        }
        let (direct, _) = &sel.parts[idx];
        let want = &sel.parts[idx - 1].1;
        if *direct {
            return match anc.split_last() {
                Some((last, rest)) => compound_matches(want, last) && up(sel, idx - 1, rest),
                None => false,
            };
        }
        for i in (0..anc.len()).rev() {
            if compound_matches(want, &anc[i]) && up(sel, idx - 1, &anc[..i]) {
                return true;
            }
        }
        false
    }
    up(sel, n - 1, anc)
}

pub struct Index<'a> {
    by_id: HashMap<&'a str, Vec<&'a Rule>>,
    by_class: HashMap<&'a str, Vec<&'a Rule>>,
    by_tag: HashMap<&'a str, Vec<&'a Rule>>,
    any: Vec<&'a Rule>,
}

impl<'a> Index<'a> {
    pub fn new(sheet: &'a Sheet) -> Index<'a> {
        let mut ix = Index { by_id: HashMap::new(), by_class: HashMap::new(), by_tag: HashMap::new(), any: Vec::new() };
        for r in &sheet.rules {
            let last = &r.selector.parts[r.selector.parts.len() - 1].1;
            if let Some(id) = &last.id {
                ix.by_id.entry(id).or_default().push(r);
            } else if let Some(c) = last.classes.first() {
                ix.by_class.entry(c).or_default().push(r);
            } else if let Some(t) = &last.tag {
                ix.by_tag.entry(t).or_default().push(r);
            } else {
                ix.any.push(r);
            }
        }
        ix
    }
}

const INHERITED: [&str; 10] = [
    "color",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "text-shadow",
    "letter-spacing",
    "text-transform",
    "line-height",
    "text-align",
];

fn compute(p: &mut Panel, anc: &mut Vec<Info>, parent: Option<&Style>, ix: &Index, sheet: &Sheet) {
    let mut hits: Vec<&Rule> = Vec::new();
    let info = Info { tag: p.tag.clone(), id: p.id.clone(), classes: p.classes.clone() };
    let sheets = p.sheets.clone();
    let mut buckets: Vec<&Vec<&Rule>> = Vec::new();
    if let Some(v) = ix.by_id.get(p.id.as_str()) {
        buckets.push(v);
    }
    for c in &p.classes {
        if let Some(v) = ix.by_class.get(c.as_str()) {
            buckets.push(v);
        }
    }
    if let Some(v) = ix.by_tag.get(p.tag.as_str()) {
        buckets.push(v);
    }
    buckets.push(&ix.any);
    for rules in buckets {
        for r in rules {
            if sheets.contains(&r.sheet) && selector_matches(&r.selector, &info, anc) {
                hits.push(r);
            }
        }
    }
    hits.sort_by_key(|r| (r.specificity, r.order));
    let mut style = Style::new();
    if let Some(par) = parent {
        for k in INHERITED {
            if let Some(v) = par.get(k) {
                style.insert(k.to_string(), v.clone());
            }
        }
    }
    for r in hits {
        for (k, v) in &r.decls {
            style.insert(k.clone(), sheet.resolve(v));
        }
    }
    for (k, v) in &p.forced {
        style.insert(k.clone(), v.clone());
    }
    p.style = style;
    let own_style = p.style.clone();
    let frame = Info { tag: p.tag.clone(), id: p.id.clone(), classes: p.classes.clone() };
    anc.push(frame);
    for c in &mut p.children {
        compute(c, anc, Some(&own_style), ix, sheet);
    }
    anc.pop();
}

pub fn apply_styles(root: &mut Panel, sheet: &Sheet) {
    let ix = Index::new(sheet);
    compute(root, &mut Vec::new(), None, &ix, sheet);
}
