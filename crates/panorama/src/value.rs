#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Len {
    Px(f32),
    Pct(f32),
    Fit,
    Fill(f32),
}

fn number(s: &str) -> Option<f32> {
    let t = s.trim().trim_end_matches("px").trim_end_matches('%').trim();
    t.parse::<f32>().ok()
}

pub fn len(v: Option<&String>) -> Len {
    let Some(v) = v else { return Len::Fit };
    let v = v.trim();
    if let Some(rest) = v.strip_prefix("fill-parent-flow") {
        let w = rest.trim().trim_start_matches('(').trim_end_matches(')').trim().parse::<f32>().unwrap_or(1.0);
        return Len::Fill(w);
    }
    if v.starts_with("fit-children") || v == "auto" || v == "fit-content" {
        return Len::Fit;
    }
    let first = v.split_whitespace().next().unwrap_or("");
    if first.ends_with('%') {
        return number(first).map_or(Len::Fit, Len::Pct);
    }
    number(first).map_or(Len::Fit, Len::Px)
}

pub fn px(v: Option<&String>) -> f32 {
    v.and_then(|v| v.split_whitespace().next()).and_then(number).unwrap_or(0.0)
}

pub fn quad(v: Option<&String>) -> Option<[f32; 4]> {
    let v = v?;
    let n: Vec<f32> = v.split_whitespace().filter_map(number).collect();
    match n.len() {
        1 => Some([n[0]; 4]),
        2 => Some([n[0], n[1], n[0], n[1]]),
        3 => Some([n[0], n[1], n[2], n[1]]),
        4 => Some([n[0], n[1], n[2], n[3]]),
        _ => None,
    }
}

pub fn hex(s: &str) -> Option<u32> {
    let h = s.trim().strip_prefix('#')?;
    let (r, g, b, a) = match h.len() {
        3 => {
            let d: Vec<u32> = h.chars().filter_map(|c| c.to_digit(16)).collect();
            (d[0] * 17, d[1] * 17, d[2] * 17, 255)
        }
        4 => {
            let d: Vec<u32> = h.chars().filter_map(|c| c.to_digit(16)).collect();
            (d[0] * 17, d[1] * 17, d[2] * 17, d[3] * 17)
        }
        6 => {
            let v = u32::from_str_radix(h, 16).ok()?;
            (v >> 16 & 255, v >> 8 & 255, v & 255, 255)
        }
        8 => {
            let v = u32::from_str_radix(h, 16).ok()?;
            (v >> 24 & 255, v >> 16 & 255, v >> 8 & 255, v & 255)
        }
        _ => return None,
    };
    Some(argb(r, g, b, a))
}

pub fn argb(r: u32, g: u32, b: u32, a: u32) -> u32 {
    (a.min(255) << 24) | (r.min(255) << 16) | (g.min(255) << 8) | b.min(255)
}

pub fn color(v: &str) -> Option<u32> {
    let v = v.trim();
    if let Some(c) = hex(v) {
        return Some(c);
    }
    if let Some(args) = v.strip_prefix("rgba(").or_else(|| v.strip_prefix("rgb(")) {
        let n: Vec<f32> = args.trim_end_matches(')').split(',').filter_map(|p| p.trim().parse().ok()).collect();
        if n.len() >= 3 {
            let a = n.get(3).copied().unwrap_or(1.0);
            return Some(argb(n[0] as u32, n[1] as u32, n[2] as u32, (a * 255.0) as u32));
        }
    }
    match v {
        "white" => Some(0xFFFFFFFF),
        "black" => Some(0xFF000000),
        "transparent" => Some(0),
        "red" => Some(0xFFFF0000),
        "green" => Some(0xFF008000),
        "blue" => Some(0xFF0000FF),
        "gray" | "grey" => Some(0xFF808080),
        "yellow" => Some(0xFFFFFF00),
        _ => None,
    }
}

pub fn with_alpha(c: u32, factor: f32) -> u32 {
    let a = ((c >> 24) as f32 * factor.clamp(0.0, 1.0)) as u32;
    (a << 24) | (c & 0x00FF_FFFF)
}

pub fn lerp_color(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let x = (a >> s & 255) as f32;
        let y = (b >> s & 255) as f32;
        (x + (y - x) * t) as u32
    };
    (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub vertical: bool,
    pub stops: Vec<(f32, u32)>,
}

fn split_args(s: &str) -> Vec<String> {
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
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

fn inner(s: &str) -> &str {
    let a = s.find('(').map_or(0, |i| i + 1);
    let b = s.rfind(')').unwrap_or(s.len());
    s[a..b.max(a)].trim()
}

pub fn gradient(v: &str) -> Option<Gradient> {
    let v = v.trim();
    if !v.starts_with("gradient") {
        return None;
    }
    let args = split_args(inner(v));
    if args.len() < 4 || args[0] != "linear" {
        return None;
    }
    let from: Vec<f32> = args[1].split_whitespace().filter_map(number).collect();
    let to: Vec<f32> = args[2].split_whitespace().filter_map(number).collect();
    if from.len() < 2 || to.len() < 2 {
        return None;
    }
    let vertical = (to[1] - from[1]).abs() >= (to[0] - from[0]).abs();
    let mut stops = Vec::new();
    for a in &args[3..] {
        if a.starts_with("from") {
            stops.push((0.0, color(inner(a))?));
        } else if a.starts_with("to") {
            stops.push((1.0, color(inner(a))?));
        } else if a.starts_with("color-stop") {
            let parts = split_args(inner(a));
            let pos = parts.first().and_then(|p| number(p))?;
            stops.push((pos, color(parts.get(1)?)?));
        }
    }
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    if stops.is_empty() {
        return None;
    }
    let (a, b) = if vertical { (from[1], to[1]) } else { (from[0], to[0]) };
    if b < a {
        stops = stops.into_iter().rev().map(|(p, c)| (1.0 - p, c)).collect();
    }
    Some(Gradient { vertical, stops })
}

pub fn url(v: &str) -> Option<String> {
    let i = v.find("url(")?;
    let rest = &v[i + 4..];
    let end = rest.find(')')?;
    Some(rest[..end].trim().trim_matches('"').trim_matches('\'').to_string())
}

pub fn translate(v: &str, w: f32, h: f32) -> (f32, f32) {
    let (mut x, mut y) = (0.0, 0.0);
    let mut rest = v;
    while let Some(i) = rest.find("translate") {
        rest = &rest[i + 9..];
        let axis = rest.chars().next().unwrap_or('(');
        let open = rest.find('(').unwrap_or(0);
        let close = rest.find(')').unwrap_or(rest.len());
        let args: Vec<&str> = rest[open + 1..close.max(open + 1)].split(',').map(str::trim).collect();
        let val = |s: &str, base: f32| -> f32 {
            if s.ends_with('%') {
                number(s).unwrap_or(0.0) * base / 100.0
            } else {
                number(s).unwrap_or(0.0)
            }
        };
        match axis {
            'X' => x += args.first().map_or(0.0, |a| val(a, w)),
            'Y' => y += args.first().map_or(0.0, |a| val(a, h)),
            _ => {
                x += args.first().map_or(0.0, |a| val(a, w));
                y += args.get(1).map_or(0.0, |a| val(a, h));
            }
        }
        rest = &rest[close.min(rest.len())..];
    }
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_and_colors_parse_the_panorama_way() {
        assert_eq!(len(Some(&"159px".to_string())), Len::Px(159.0));
        assert_eq!(len(Some(&"100%".to_string())), Len::Pct(100.0));
        assert_eq!(len(Some(&"fill-parent-flow( 2.0 )".to_string())), Len::Fill(2.0));
        assert_eq!(len(Some(&"fit-children".to_string())), Len::Fit);
        assert_eq!(hex("#ff000080"), Some(argb(255, 0, 0, 128)));
        assert_eq!(color("#1620346c"), Some(argb(0x16, 0x20, 0x34, 0x6c)));
        assert_eq!(quad(Some(&"1px 2px".to_string())), Some([1.0, 2.0, 1.0, 2.0]));
    }

    #[test]
    fn gradients_and_translations_are_read() {
        let g = gradient("gradient( linear, 0% 0%, 100% 0%, from( #00000000 ), color-stop( .4, #00000000 ), to( #000000ff ) )").unwrap();
        assert!(!g.vertical);
        assert_eq!(g.stops.len(), 3);
        assert_eq!(url("url(\"s2r://x/y_psd.vtex\")").as_deref(), Some("s2r://x/y_psd.vtex"));
        assert_eq!(translate("translateY( 14px ) translateX( -16px )", 100.0, 100.0), (-16.0, 14.0));
    }
}
