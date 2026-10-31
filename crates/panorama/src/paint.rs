use crate::layout::{collapsed, font_of, line_height, Ctx};
use crate::tree::Panel;
use crate::value::{color, gradient, lerp_color, px, url, with_alpha, Gradient};
use icraft_ui::{Click, Fill, Image, Label, Op, Paint};

pub type Resolve<'a> = &'a dyn Fn(&str) -> Option<(String, f32, f32)>;

pub struct Out<'a> {
    pub k: f32,
    pub resolve: Resolve<'a>,
    pub paint: Paint,
    pub current: String,
    pub trace: Vec<String>,
}

impl Out<'_> {
    fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, c: u32) {
        if w <= 0.0 || h <= 0.0 || c >> 24 == 0 {
            return;
        }
        let f = Fill { x: x * self.k, y: y * self.k, w: w * self.k, h: h * self.k, color: c };
        self.paint.fills.push(f.clone());
        self.paint.ops.push(Op::Fill(f));
        self.trace.push(self.current.clone());
    }

    fn image(&mut self, x: f32, y: f32, w: f32, h: f32, src: &str, opacity: f32) {
        if w <= 0.0 || h <= 0.0 || opacity <= 0.0 {
            return;
        }
        let i = Image { x: x * self.k, y: y * self.k, w: w * self.k, h: h * self.k, src: src.to_string(), opacity };
        self.paint.images.push(i.clone());
        self.paint.ops.push(Op::Image(i));
        self.trace.push(self.current.clone());
    }
}

fn split_top(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

fn gradient_fill(out: &mut Out, g: &Gradient, x: f32, y: f32, w: f32, h: f32, opacity: f32) {
    let span = if g.vertical { h } else { w };
    let steps = ((span * out.k / 4.0).ceil() as usize).clamp(1, 64);
    for i in 0..steps {
        let t = (i as f32 + 0.5) / steps as f32;
        let mut c = g.stops[0].1;
        if t >= g.stops[g.stops.len() - 1].0 {
            c = g.stops[g.stops.len() - 1].1;
        } else {
            for pair in g.stops.windows(2) {
                if t >= pair[0].0 && t <= pair[1].0 {
                    let d = (pair[1].0 - pair[0].0).max(1e-4);
                    c = lerp_color(pair[0].1, pair[1].1, (t - pair[0].0) / d);
                    break;
                }
            }
        }
        let c = with_alpha(c, opacity);
        let (a, b) = (i as f32 / steps as f32, (i + 1) as f32 / steps as f32);
        if g.vertical {
            out.fill(x, y + h * a, w, h * (b - a) + 0.25, c);
        } else {
            out.fill(x + w * a, y, w * (b - a) + 0.25, h, c);
        }
    }
}

fn pos_axis(v: &str, free: f32) -> f32 {
    match v {
        "left" | "top" => 0.0,
        "center" | "middle" => free / 2.0,
        "right" | "bottom" => free,
        _ if v.ends_with('%') => free * v.trim_end_matches('%').parse::<f32>().unwrap_or(50.0) / 100.0,
        _ => v.trim_end_matches("px").parse::<f32>().unwrap_or(0.0),
    }
}

fn background_images(p: &Panel, out: &mut Out, opacity: f32) {
    let Some(urls) = p.style.get("background-image") else { return };
    let sizes = p.style.get("background-size").map(|s| split_top(s)).unwrap_or_default();
    let poses = p.style.get("background-position").map(|s| split_top(s)).unwrap_or_default();
    let repeats = p.style.get("background-repeat").map(|s| split_top(s)).unwrap_or_default();
    for (i, entry) in split_top(urls).iter().enumerate().rev() {
        let Some(u) = url(entry) else { continue };
        let Some((name, iw, ih)) = (out.resolve)(&u) else { continue };
        let r = &p.rect;
        let size = sizes.get(i).or(sizes.first()).map(String::as_str).unwrap_or("auto");
        let toks: Vec<&str> = size.split_whitespace().collect();
        let (mut dw, mut dh) = (iw, ih);
        let dim = |t: &str, base: f32| -> Option<f32> {
            if t.ends_with('%') {
                t.trim_end_matches('%').parse::<f32>().ok().map(|v| base * v / 100.0)
            } else if t == "auto" {
                None
            } else {
                t.trim_end_matches("px").parse::<f32>().ok()
            }
        };
        match toks.first().copied().unwrap_or("auto") {
            "contain" => {
                let s = (r.w / iw).min(r.h / ih);
                dw = iw * s;
                dh = ih * s;
            }
            "cover" => {
                let s = (r.w / iw).max(r.h / ih);
                dw = iw * s;
                dh = ih * s;
            }
            t => {
                let w = dim(t, r.w);
                let h = toks.get(1).and_then(|t| dim(t, r.h));
                match (w, h) {
                    (Some(w), Some(h)) => {
                        dw = w;
                        dh = h;
                    }
                    (Some(w), None) => {
                        dw = w;
                        dh = if toks.get(1).is_none() || toks[1] == "auto" { ih * w / iw } else { ih };
                    }
                    (None, Some(h)) => {
                        dh = h;
                        dw = iw * h / ih;
                    }
                    (None, None) => {}
                }
            }
        }
        let pos = poses.get(i).or(poses.first()).map(String::as_str).unwrap_or("50% 50%");
        let pt: Vec<&str> = pos.split_whitespace().collect();
        let (hx, hy) = match pt.len() {
            0 => ("50%", "50%"),
            1 => {
                if matches!(pt[0], "top" | "bottom") {
                    ("50%", pt[0])
                } else {
                    (pt[0], "50%")
                }
            }
            _ => (pt[0], pt[1]),
        };
        let ox = pos_axis(hx, r.w - dw);
        let oy = pos_axis(hy, r.h - dh);
        let repeat = repeats.get(i).or(repeats.first()).map(String::as_str).unwrap_or("no-repeat");
        let (rx, ry) = (repeat.contains("repeat-x") || repeat == "repeat", repeat.contains("repeat-y") || repeat == "repeat");
        if (rx || ry) && dw > 0.5 && dh > 0.5 {
            let nx = if rx { ((r.w / dw).ceil() as usize).clamp(1, 64) } else { 1 };
            let ny = if ry { ((r.h / dh).ceil() as usize).clamp(1, 64) } else { 1 };
            for yy in 0..ny {
                for xx in 0..nx {
                    out.image(r.x + xx as f32 * dw, r.y + yy as f32 * dh, dw, dh, &name, opacity);
                }
            }
        } else {
            out.image(r.x + ox, r.y + oy, dw, dh, &name, opacity);
        }
    }
}

fn borders(p: &Panel, out: &mut Out, opacity: f32) {
    let r = &p.rect;
    let mut sides: [Option<(f32, u32)>; 4] = [None; 4];
    let parse = |v: &str| -> Option<(f32, u32)> {
        let mut width = 1.0;
        let mut col = None;
        for t in v.split_whitespace() {
            if let Some(c) = color(t) {
                col = Some(c);
            } else if t.ends_with("px") {
                width = t.trim_end_matches("px").parse().unwrap_or(1.0);
            }
        }
        col.map(|c| (width, c))
    };
    if let Some(v) = p.style.get("border") {
        let s = parse(v);
        sides = [s; 4];
    }
    for (i, k) in ["border-top", "border-right", "border-bottom", "border-left"].iter().enumerate() {
        if let Some(v) = p.style.get(*k) {
            sides[i] = parse(v);
        }
    }
    for (i, k) in ["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"].iter().enumerate() {
        if let (Some(v), Some((w, _))) = (p.style.get(*k), sides[i]) {
            if let Some(c) = color(v) {
                sides[i] = Some((w, c));
            }
        }
    }
    if let Some((w, c)) = sides[0] {
        out.fill(r.x, r.y, r.w, w, with_alpha(c, opacity));
    }
    if let Some((w, c)) = sides[2] {
        out.fill(r.x, r.y + r.h - w, r.w, w, with_alpha(c, opacity));
    }
    if let Some((w, c)) = sides[3] {
        out.fill(r.x, r.y, w, r.h, with_alpha(c, opacity));
    }
    if let Some((w, c)) = sides[1] {
        out.fill(r.x + r.w - w, r.y, w, r.h, with_alpha(c, opacity));
    }
}

fn label(p: &Panel, ctx: &Ctx, out: &mut Out, opacity: f32) {
    let mut text = (ctx.text)(p);
    if text.is_empty() {
        return;
    }
    if p.style.get("text-transform").is_some_and(|v| v.trim() == "uppercase") {
        text = text.to_uppercase();
    }
    let font = font_of(p);
    let width = (ctx.measure)(&text, &font);
    let r = &p.rect;
    let x = match p.style.get("text-align").map(|s| s.trim()) {
        Some("right") => r.x + r.w - width,
        Some("center") => r.x + (r.w - width) / 2.0,
        _ => r.x,
    };
    let lh = line_height(p);
    let y = r.y + ((r.h - lh) / 2.0).max(0.0);
    let col = p.style.get("color").and_then(|c| color(c)).unwrap_or(0xFFFFFFFF);
    let shadow = p
        .style
        .get("text-shadow")
        .and_then(|s| s.split_whitespace().rev().find_map(color))
        .map(|c| with_alpha(c, opacity));
    let l = Label {
        x: x * out.k,
        y: y * out.k,
        text,
        color: with_alpha(col, opacity),
        bold: font.weight >= 600,
        scale: 1.0,
        family: font.family.clone(),
        size: font.size * out.k,
        weight: font.weight,
        shadow,
        line_height: lh * out.k,
    };
    out.paint.labels.push(l.clone());
    out.paint.ops.push(Op::Label(l));
    out.trace.push(out.current.clone());
}

pub fn paint(p: &Panel, ctx: &Ctx, out: &mut Out, inherited: f32) {
    if collapsed(p) || p.style.get("visibility").is_some_and(|v| v.trim() == "hidden") {
        return;
    }
    let opacity = inherited * p.style.get("opacity").and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(1.0);
    if opacity <= 0.0 {
        return;
    }
    let r = p.rect.clone();
    out.current = format!("{}#{}.{}", p.tag, p.id, p.classes.join("."));
    if let Some(bg) = p.style.get("background-color") {
        if let Some(g) = gradient(bg) {
            gradient_fill(out, &g, r.x, r.y, r.w, r.h, opacity);
        } else if let Some(c) = color(bg) {
            out.fill(r.x, r.y, r.w, r.h, with_alpha(c, opacity));
        }
    }
    background_images(p, out, opacity);
    borders(p, out, opacity);
    match p.tag.as_str() {
        "Label" => label(p, ctx, out, opacity),
        "Image" | "DOTAAbilityImage" | "DOTAItemImage" | "DOTAPortrait" | "DOTAHeroImage" | "DOTAMinimap" | "DOTAShopItem" => {
            if p.image.is_empty() {
            } else if let Some((name, iw, ih)) = (out.resolve)(&p.image) {
                let preserve = p.attr("scaling").is_some_and(|s| s.contains("preserve"));
                if preserve && iw > 0.0 && ih > 0.0 {
                    let s = (r.w / iw).min(r.h / ih);
                    let (w, h) = (iw * s, ih * s);
                    out.image(r.x + (r.w - w) / 2.0, r.y + (r.h - h) / 2.0, w, h, &name, opacity);
                } else {
                    out.image(r.x, r.y, r.w, r.h, &name, opacity);
                }
            }
        }
        _ => {}
    }
    if !p.click.is_empty() {
        out.paint.clicks.push(Click { x: r.x * out.k, y: r.y * out.k, w: r.w * out.k, h: r.h * out.k, onclick: p.click.clone() });
    }
    let mut order: Vec<usize> = (0..p.children.len()).collect();
    order.sort_by_key(|&i| p.children[i].style.get("z-index").map_or(0, |z| px(Some(z)) as i32));
    for i in order {
        paint_child(&p.children[i], ctx, out, opacity);
    }
}

fn paint_child(c: &Panel, ctx: &Ctx, out: &mut Out, opacity: f32) {
    paint(c, ctx, out, opacity);
}
