use crate::tree::Panel;
use crate::value::{len, px, quad, translate, Len};

pub struct Ctx<'a> {
    pub measure: &'a dyn Fn(&str, &icraft_ui::Font) -> f32,
    pub image_size: &'a dyn Fn(&str) -> Option<(f32, f32)>,
    pub text: &'a dyn Fn(&Panel) -> String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Flow {
    None,
    Right,
    Left,
    Down,
    Up,
}

fn flow(p: &Panel) -> Flow {
    match p.style.get("flow-children").map(|s| s.trim()) {
        Some(v) if v.starts_with("right") => Flow::Right,
        Some(v) if v.starts_with("left") => Flow::Left,
        Some(v) if v.starts_with("down") => Flow::Down,
        Some(v) if v.starts_with("up") => Flow::Up,
        _ => Flow::None,
    }
}

pub fn collapsed(p: &Panel) -> bool {
    p.hidden
        || p.style.get("visibility").is_some_and(|v| v.trim() == "collapse")
        || p.style.get("display").is_some_and(|v| v.trim() == "none")
}

fn margins(p: &Panel) -> [f32; 4] {
    let mut m = quad(p.style.get("margin")).unwrap_or([0.0; 4]);
    for (i, k) in ["margin-top", "margin-right", "margin-bottom", "margin-left"].iter().enumerate() {
        if let Some(v) = p.style.get(*k) {
            m[i] = px(Some(v));
        }
    }
    m
}

fn paddings(p: &Panel) -> [f32; 4] {
    let mut m = quad(p.style.get("padding")).unwrap_or([0.0; 4]);
    for (i, k) in ["padding-top", "padding-right", "padding-bottom", "padding-left"].iter().enumerate() {
        if let Some(v) = p.style.get(*k) {
            m[i] = px(Some(v));
        }
    }
    m
}

pub fn font_of(p: &Panel) -> icraft_ui::Font {
    let family = match p.style.get("font-family").map(|s| s.trim().trim_matches('\'').trim_matches('"')) {
        Some("titleFont") | Some("reaver") | Some("Reaver") => "Reaver",
        Some("monospaceNumbersFont") | Some("monospaceFont") | Some("RadianceM") => "RadianceM",
        _ => "Radiance",
    };
    let size = p.style.get("font-size").map_or(18.0, |v| px(Some(v)).max(1.0));
    let weight = match p.style.get("font-weight").map(|s| s.trim()) {
        Some("bold") => 700,
        Some("light") => 300,
        Some("medium") => 500,
        Some("black") => 900,
        Some("semi-bold") | Some("semibold") => 600,
        Some(n) => n.parse().unwrap_or(400),
        None => 400,
    };
    icraft_ui::Font { family: family.to_string(), size, weight }
}

pub fn line_height(p: &Panel) -> f32 {
    let size = font_of(p).size;
    match p.style.get("line-height") {
        Some(v) => {
            let n = px(Some(v));
            if v.contains('%') {
                size * n / 100.0
            } else if n > 0.0 {
                n
            } else {
                size * 1.2
            }
        }
        None => size * 1.2,
    }
}

fn intrinsic(p: &Panel, ctx: &Ctx) -> Option<(f32, f32)> {
    match p.tag.as_str() {
        "Label" => {
            let text = (ctx.text)(p);
            let font = font_of(p);
            let text = if p.style.get("text-transform").is_some_and(|v| v.trim() == "uppercase") { text.to_uppercase() } else { text };
            Some(((ctx.measure)(&text, &font), line_height(p)))
        }
        "Image" => (ctx.image_size)(&p.image),
        _ => None,
    }
}

pub fn size_of(p: &Panel, pw: Option<f32>, ph: Option<f32>, ctx: &Ctx) -> (f32, f32) {
    let wl = len(p.style.get("width"));
    let hl = len(p.style.get("height"));
    let m = margins(p);
    let known = |l: Len, avail: Option<f32>, lead: f32, trail: f32| match l {
        Len::Px(v) => Some(v),
        Len::Pct(v) => avail.map(|a| ((a - lead - trail).max(0.0)) * v / 100.0),
        Len::Fill(_) => avail,
        Len::Fit => None,
    };
    let (wk, hk) = (known(wl, pw, m[3], m[1]), known(hl, ph, m[0], m[2]));
    let pad = paddings(p);
    let (mut w, mut h);
    if let (Some(a), Some(b)) = (wk, hk) {
        w = a;
        h = b;
    } else {
        let cw = wk.map(|v| v - pad[1] - pad[3]);
        let ch = hk.map(|v| v - pad[0] - pad[2]);
        let (fw, fh) = content_size(p, cw, ch, ctx);
        w = wk.unwrap_or(fw + pad[1] + pad[3]);
        h = hk.unwrap_or(fh + pad[0] + pad[2]);
    }
    let clamp = |v: f32, lo: &str, hi: &str, avail: Option<f32>| {
        let mut v = v;
        let get = |k: &str| -> Option<f32> {
            match len(p.style.get(k)) {
                Len::Px(x) => Some(x),
                Len::Pct(x) => avail.map(|a| a * x / 100.0),
                _ => None,
            }
        };
        if let Some(m) = get(hi) {
            v = v.min(m);
        }
        if let Some(m) = get(lo) {
            v = v.max(m);
        }
        v
    };
    w = clamp(w, "min-width", "max-width", pw);
    h = clamp(h, "min-height", "max-height", ph);
    (w.max(0.0), h.max(0.0))
}

fn content_size(p: &Panel, cw: Option<f32>, ch: Option<f32>, ctx: &Ctx) -> (f32, f32) {
    if let Some(size) = intrinsic(p, ctx) {
        return size;
    }
    let f = flow(p);
    let (mut w, mut h) = (0.0f32, 0.0f32);
    for c in p.children.iter().filter(|c| !collapsed(c)) {
        let m = margins(c);
        let wl = len(c.style.get("width"));
        let hl = len(c.style.get("height"));
        let (mut sw, mut sh) = size_of(c, cw, ch, ctx);
        if matches!(f, Flow::Right | Flow::Left) && matches!(wl, Len::Fill(_)) {
            sw = 0.0;
        }
        if matches!(f, Flow::Down | Flow::Up) && matches!(hl, Len::Fill(_)) {
            sh = 0.0;
        }
        let ew = sw + m[1] + m[3];
        let eh = sh + m[0] + m[2];
        match f {
            Flow::None => {
                w = w.max(ew);
                h = h.max(eh);
            }
            Flow::Right | Flow::Left => {
                w += ew;
                h = h.max(eh);
            }
            Flow::Down | Flow::Up => {
                w = w.max(ew);
                h += eh;
            }
        }
    }
    (w, h)
}

fn align_h(p: &Panel) -> &str {
    p.style.get("horizontal-align").map_or("left", |s| s.trim())
}

fn align_v(p: &Panel) -> &str {
    p.style.get("vertical-align").map_or("top", |s| s.trim())
}

fn offset_of(p: &Panel, w: f32, h: f32) -> (f32, f32) {
    let (mut x, mut y) = (0.0, 0.0);
    if let Some(pos) = p.style.get("position") {
        let n: Vec<&str> = pos.split_whitespace().collect();
        let val = |s: &str| px(Some(&s.to_string()));
        if n.len() >= 2 {
            x += val(n[0]);
            y += val(n[1]);
        }
    }
    if let Some(t) = p.style.get("transform") {
        let (tx, ty) = translate(t, w, h);
        x += tx;
        y += ty;
    }
    (x, y)
}

pub fn arrange(p: &mut Panel, x: f32, y: f32, w: f32, h: f32, ctx: &Ctx) {
    p.rect.x = x;
    p.rect.y = y;
    p.rect.w = w;
    p.rect.h = h;
    let pad = paddings(p);
    let (cx, cy) = (x + pad[3], y + pad[0]);
    let (cw, ch) = ((w - pad[1] - pad[3]).max(0.0), (h - pad[0] - pad[2]).max(0.0));
    let f = flow(p);
    let n = p.children.len();
    let mut sizes: Vec<(f32, f32)> = Vec::with_capacity(n);
    let mut main_used = 0.0f32;
    let mut fill_weight = 0.0f32;
    for c in &p.children {
        if collapsed(c) {
            sizes.push((0.0, 0.0));
            continue;
        }
        let m = margins(c);
        let (mut sw, mut sh) = size_of(c, Some(cw), Some(ch), ctx);
        let wl = len(c.style.get("width"));
        let hl = len(c.style.get("height"));
        match f {
            Flow::Right | Flow::Left => {
                if let Len::Fill(weight) = wl {
                    fill_weight += weight;
                    sw = 0.0;
                }
                main_used += sw + m[1] + m[3];
            }
            Flow::Down | Flow::Up => {
                if let Len::Fill(weight) = hl {
                    fill_weight += weight;
                    sh = 0.0;
                }
                main_used += sh + m[0] + m[2];
            }
            Flow::None => {}
        }
        sizes.push((sw, sh));
    }
    if fill_weight > 0.0 {
        let free = match f {
            Flow::Right | Flow::Left => cw - main_used,
            _ => ch - main_used,
        }
        .max(0.0);
        for (i, c) in p.children.iter().enumerate() {
            if collapsed(c) {
                continue;
            }
            match f {
                Flow::Right | Flow::Left => {
                    if let Len::Fill(weight) = len(c.style.get("width")) {
                        sizes[i].0 = free * weight / fill_weight;
                    }
                }
                _ => {
                    if let Len::Fill(weight) = len(c.style.get("height")) {
                        sizes[i].1 = free * weight / fill_weight;
                    }
                }
            }
        }
    }
    let mut cursor = match f {
        Flow::Left => cx + cw,
        Flow::Up => cy + ch,
        _ => 0.0,
    };
    let mut cursor_main = match f {
        Flow::Right => cx,
        Flow::Down => cy,
        _ => cursor,
    };
    for (i, c) in p.children.iter_mut().enumerate() {
        if collapsed(c) {
            continue;
        }
        let m = margins(c);
        let (sw, sh) = sizes[i];
        let (ah, av) = (align_h(c).to_string(), align_v(c).to_string());
        let place_h = |m: [f32; 4]| match ah.as_str() {
            "center" | "middle" => cx + m[3] + (cw - m[1] - m[3] - sw) / 2.0,
            "right" => cx + cw - m[1] - sw,
            _ => cx + m[3],
        };
        let place_v = |m: [f32; 4]| match av.as_str() {
            "center" | "middle" => cy + m[0] + (ch - m[0] - m[2] - sh) / 2.0,
            "bottom" => cy + ch - m[2] - sh,
            _ => cy + m[0],
        };
        let (px_, py_) = match f {
            Flow::None => (place_h(m), place_v(m)),
            Flow::Right => {
                let x0 = cursor_main + m[3];
                cursor_main = x0 + sw + m[1];
                (x0, place_v(m))
            }
            Flow::Left => {
                let x0 = cursor_main - m[1] - sw;
                cursor_main = x0 - m[3];
                (x0, place_v(m))
            }
            Flow::Down => {
                let y0 = cursor_main + m[0];
                cursor_main = y0 + sh + m[2];
                (place_h(m), y0)
            }
            Flow::Up => {
                let y0 = cursor_main - m[2] - sh;
                cursor_main = y0 - m[0];
                (place_h(m), y0)
            }
        };
        let _ = &mut cursor;
        let (ox, oy) = offset_of(c, sw, sh);
        arrange(c, px_ + ox, py_ + oy, sw, sh, ctx);
    }
}
