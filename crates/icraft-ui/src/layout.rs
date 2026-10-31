//! Lays out an I-Craft window the way a browser does for the CSS subset the
//! server documents use: block and flex boxes (row or column, wrap, gap,
//! flex-grow, align-items, justify-content), width, height, min-width and
//! max-width in px or %, padding, margin, background, border, inherited
//! color, font-weight and `:hover`. Text wraps by words. The body centres its
//! children on screen.
//!
//! Selectors: tag, `.class`, `#id`, compounds (`.foot.msg`), descendants
//! (`.card .name`), `:hover` on any part, comma lists. Later and more
//! specific rules win, as in CSS.

use crate::html::{parse, Element, Node};

/// One text line: the 8 px font plus spacing.
pub const LINE: f32 = 10.0;
/// An `<item>` icon.
pub const ITEM: f32 = 16.0;

#[derive(Clone, Debug)]
pub struct Fill {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: u32,
}

#[derive(Clone, Debug)]
pub struct Label {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub color: u32,
    pub bold: bool,
    pub scale: f32,
    pub family: String,
    pub size: f32,
    pub weight: u16,
    pub shadow: Option<u32>,
    pub line_height: f32,
}

#[derive(Clone, Debug)]
pub struct Image {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub src: String,
    pub opacity: f32,
}

#[derive(Clone, Debug)]
pub struct Gradient {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub top: u32,
    pub bottom: u32,
}

#[derive(Clone, Debug)]
pub enum Op {
    Fill(Fill),
    Gradient(Gradient),
    Image(Image),
    Label(Label),
    Icon(Icon),
}

#[derive(Clone, Debug)]
pub struct Icon {
    pub x: f32,
    pub y: f32,
    pub item: String,
}

#[derive(Clone, Debug)]
pub struct Click {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub onclick: String,
}

/// What to draw, in order: fills, then labels, then icons.
#[derive(Clone, Debug, Default)]
pub struct Paint {
    pub fills: Vec<Fill>,
    pub labels: Vec<Label>,
    pub icons: Vec<Icon>,
    pub clicks: Vec<Click>,
    pub images: Vec<Image>,
    pub gradients: Vec<Gradient>,
    /// Every drawing command in paint order: a box's background, its image,
    /// its border, then its children.
    pub ops: Vec<Op>,
    /// The body's children, for "a click outside the window closes it".
    pub windows: Vec<(f32, f32, f32, f32)>,
}

/// Text width in GUI pixels; `bold` text is wider.
pub type Measure<'a> = &'a dyn Fn(&str, bool) -> f32;

#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub family: String,
    pub size: f32,
    pub weight: u16,
}

pub type RichMeasure<'a> = &'a dyn Fn(&str, &Font) -> f32;

/// Lays out with an approximate font (6 px per character), for tests and
/// callers without a font.
pub fn paint(html: &str, css: &str, width: f32, height: f32, mouse_x: f32, mouse_y: f32) -> Paint {
    let measure = |text: &str, bold: bool| text.chars().count() as f32 * if bold { 7.0 } else { 6.0 };
    paint_with(html, css, width, height, mouse_x, mouse_y, &measure)
}

pub fn paint_with(html: &str, css: &str, width: f32, height: f32, mouse_x: f32, mouse_y: f32, measure: Measure) -> Paint {
    paint_rich(html, css, width, height, mouse_x, mouse_y, measure, None)
}

#[allow(clippy::too_many_arguments)]
pub fn paint_rich(html: &str, css: &str, width: f32, height: f32, mouse_x: f32, mouse_y: f32, measure: Measure, rich: Option<RichMeasure>) -> Paint {
    let Ok(root) = parse(html) else { return Paint::default() };
    let sheet = Sheet::parse(css);
    let ctx = Ctx { sheet: &sheet, measure, rich };
    let root_style = Style::default();
    let mut boxes = Vec::new();
    let mut total = 0.0;
    for child in &root.children {
        let Node::Element(element) = child else { continue };
        let chain = [element];
        let style = ctx.style(&chain, &root_style);
        if style.hidden {
            continue;
        }
        let w = ctx.outer_width(element, &chain, &style, width, true);
        let b = ctx.layout(element, &chain, &style, w);
        total += b.h + style.margin[0] + style.margin[2];
        boxes.push((b, style));
    }
    let mut out = Paint::default();
    let mut y = ((height - total) / 2.0).max(0.0);
    for (b, style) in &boxes {
        y += style.margin[0];
        let x = ((width - b.w) / 2.0).max(0.0);
        out.windows.push((x, y, b.w, b.h));
        let mut chain = Vec::new();
        ctx.paint(b, x, y, &mut chain, &root_style, 1.0, (mouse_x, mouse_y), &mut out);
        y += b.h + style.margin[2];
    }
    out
}

pub fn click_at(paint: &Paint, x: f32, y: f32) -> Option<&str> {
    paint.clicks.iter().rev().find(|c| x >= c.x && y >= c.y && x < c.x + c.w && y < c.y + c.h).map(|c| c.onclick.as_str())
}

/// Whether a point is inside one of the window boxes.
pub fn inside(paint: &Paint, x: f32, y: f32) -> bool {
    paint.windows.iter().any(|&(wx, wy, ww, wh)| x >= wx && y >= wy && x < wx + ww && y < wy + wh)
}

// ---- styles ----

#[derive(Clone, Copy, Debug, PartialEq)]
enum Len {
    Px(f32),
    Pct(f32),
}

impl Len {
    fn of(self, avail: f32) -> f32 {
        match self {
            Len::Px(v) => v,
            Len::Pct(p) => avail * p / 100.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Align {
    Stretch,
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Justify {
    Start,
    Center,
    End,
    Between,
}

#[derive(Clone, Debug)]
struct Style {
    flex: bool,
    row: bool,
    wrap: bool,
    hidden: bool,
    grow: f32,
    width: Option<Len>,
    height: Option<f32>,
    min_width: Option<f32>,
    max_width: Option<Len>,
    pad: [f32; 4],
    margin: [f32; 4],
    gap: f32,
    align: Align,
    justify: Justify,
    bg: Option<u32>,
    border: Option<u32>,
    absolute: bool,
    left: Option<Len>,
    top: Option<Len>,
    right: Option<Len>,
    bottom: Option<Len>,
    height_pct: Option<f32>,
    opacity: f32,
    gradient: Option<(u32, u32)>,
    bg_image: Option<String>,
    // Inherited.
    text_center: bool,
    text_right: bool,
    color: u32,
    bold: bool,
    scale: f32,
    family: String,
    size: f32,
    weight: u16,
    upper: bool,
    shadow: Option<u32>,
    line_height: Option<f32>,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            flex: false,
            // flex-direction defaults to row; only flex boxes look at it.
            row: true,
            wrap: false,
            hidden: false,
            grow: 0.0,
            width: None,
            height: None,
            min_width: None,
            max_width: None,
            pad: [0.0; 4],
            margin: [0.0; 4],
            gap: 0.0,
            align: Align::Stretch,
            justify: Justify::Start,
            bg: None,
            border: None,
            absolute: false,
            left: None,
            top: None,
            right: None,
            bottom: None,
            height_pct: None,
            opacity: 1.0,
            gradient: None,
            bg_image: None,
            text_center: false,
            text_right: false,
            color: 0xFFF2_F2F5,
            bold: false,
            scale: 1.0,
            family: String::new(),
            size: 0.0,
            weight: 400,
            upper: false,
            shadow: None,
            line_height: None,
        }
    }
}

impl Style {
    fn inherit(parent: &Style) -> Style {
        Style { color: parent.color, bold: parent.bold, text_center: parent.text_center, text_right: parent.text_right, scale: parent.scale, family: parent.family.clone(), size: parent.size, weight: parent.weight, upper: parent.upper, shadow: parent.shadow, line_height: parent.line_height, ..Style::default() }
    }

    fn set(&mut self, name: &str, value: &str) {
        match name {
            "display" => {
                self.hidden = value == "none";
                self.flex = value == "flex" || value == "inline-flex";
            }
            "flex-direction" => self.row = !value.starts_with("column"),
            "flex-wrap" => self.wrap = value == "wrap",
            "flex-grow" => self.grow = value.parse().unwrap_or(0.0),
            "flex" => self.grow = value.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0.0),
            "width" => self.width = len(value),
            "height" => match len(value) {
                Some(Len::Px(v)) => {
                    self.height = Some(v);
                    self.height_pct = None;
                }
                Some(Len::Pct(p)) => {
                    self.height_pct = Some(p);
                    self.height = None;
                }
                None => {}
            },
            "position" => self.absolute = value == "absolute",
            "left" => self.left = len(value),
            "top" => self.top = len(value),
            "right" => self.right = len(value),
            "bottom" => self.bottom = len(value),
            "opacity" => self.opacity = value.parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0),
            "font-size" => {
                if let Some(v) = px(value) {
                    self.scale = (v / 8.0).clamp(0.5, 8.0);
                    self.size = v.max(1.0);
                }
            }
            "font-family" => {
                self.family = value.split(',').next().unwrap_or("").trim().trim_matches(|c| c == '"' || c == '\'').to_owned();
            }
            "text-transform" => self.upper = value == "uppercase",
            "text-shadow" => self.shadow = value.split_whitespace().find_map(color).or(self.shadow),
            "line-height" => self.line_height = px(value),
            "min-width" => self.min_width = px(value),
            "max-width" => self.max_width = len(value),
            "gap" => self.gap = px(value).unwrap_or(0.0),
            "padding" => self.pad = sides(value),
            "padding-top" => self.pad[0] = px(value).unwrap_or(0.0),
            "padding-right" => self.pad[1] = px(value).unwrap_or(0.0),
            "padding-bottom" => self.pad[2] = px(value).unwrap_or(0.0),
            "padding-left" => self.pad[3] = px(value).unwrap_or(0.0),
            "margin" => self.margin = sides(value),
            "margin-top" => self.margin[0] = px(value).unwrap_or(0.0),
            "margin-right" => self.margin[1] = px(value).unwrap_or(0.0),
            "margin-bottom" => self.margin[2] = px(value).unwrap_or(0.0),
            "margin-left" => self.margin[3] = px(value).unwrap_or(0.0),
            "align-items" => {
                self.align = match value {
                    "center" => Align::Center,
                    "flex-start" | "start" => Align::Start,
                    "flex-end" | "end" => Align::End,
                    _ => Align::Stretch,
                }
            }
            "justify-content" => {
                self.justify = match value {
                    "center" => Justify::Center,
                    "flex-end" | "end" => Justify::End,
                    "space-between" => Justify::Between,
                    _ => Justify::Start,
                }
            }
            "text-align" => {
                self.text_center = value == "center";
                self.text_right = value == "right";
            }
            "background-color" => self.bg = color(value),
            "background" | "background-image" => {
                if let Some(pair) = gradient(value) {
                    self.gradient = Some(pair);
                } else if let Some(url) = url_of(value) {
                    self.bg_image = Some(url);
                    if let Some(c) = value.split_whitespace().find_map(color) {
                        self.bg = Some(c);
                    }
                } else if name == "background" {
                    self.bg = color(value);
                }
            }
            "border" => self.border = value.split_whitespace().find_map(color),
            "border-color" => self.border = color(value),
            "color" => {
                if let Some(c) = color(value) {
                    self.color = c;
                }
            }
            "font-weight" => {
                self.bold = value == "bold" || value.parse::<u32>().is_ok_and(|w| w >= 600);
                self.weight = match value {
                    "bold" => 700,
                    "normal" => 400,
                    other => other.parse().unwrap_or(400),
                };
            }
            _ => {}
        }
    }
}

struct Rule {
    /// Compound selectors from the outermost ancestor to the subject.
    parts: Vec<Compound>,
    decls: Vec<(String, String)>,
}

#[derive(Default)]
struct Compound {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    hover: bool,
}

impl Compound {
    fn parse(text: &str) -> Compound {
        let mut c = Compound::default();
        let text = match text.strip_suffix(":hover") {
            Some(rest) => {
                c.hover = true;
                rest
            }
            None => text,
        };
        let mut cur = String::new();
        let mut kind = 't';
        let push = |kind: char, cur: &mut String, c: &mut Compound| {
            let part = std::mem::take(cur);
            if part.is_empty() || part == "*" {
                return;
            }
            match kind {
                '.' => c.classes.push(part),
                '#' => c.id = Some(part),
                _ => c.tag = Some(part.to_ascii_lowercase()),
            }
        };
        for ch in text.chars() {
            if ch == '.' || ch == '#' {
                push(kind, &mut cur, &mut c);
                kind = ch;
            } else {
                cur.push(ch);
            }
        }
        push(kind, &mut cur, &mut c);
        c
    }

    fn specificity(&self) -> u32 {
        self.id.is_some() as u32 * 100 + (self.classes.len() as u32 + self.hover as u32) * 10 + self.tag.is_some() as u32
    }

    fn matches(&self, element: &Element, hovered: bool) -> bool {
        (!self.hover || hovered)
            && self.tag.as_ref().is_none_or(|t| *t == element.tag)
            && self.id.as_ref().is_none_or(|id| element.attr("id") == Some(id.as_str()))
            && self.classes.iter().all(|class| element.has_class(class))
    }
}

struct Sheet {
    rules: Vec<Rule>,
}

impl Sheet {
    fn parse(css: &str) -> Sheet {
        let mut rules: Vec<(u32, usize, Rule)> = Vec::new();
        for chunk in strip_comments(css).split('}') {
            let Some((selectors, body)) = chunk.split_once('{') else { continue };
            let decls: Vec<(String, String)> = body
                .split(';')
                .filter_map(|d| d.split_once(':'))
                .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().trim_end_matches("!important").trim().to_owned()))
                .collect();
            for selector in selectors.split(',') {
                let parts: Vec<Compound> = selector.split_whitespace().map(Compound::parse).collect();
                if parts.is_empty() {
                    continue;
                }
                let specificity = parts.iter().map(Compound::specificity).sum();
                let order = rules.len();
                rules.push((specificity, order, Rule { parts, decls: decls.clone() }));
            }
        }
        rules.sort_by_key(|(specificity, order, _)| (*specificity, *order));
        Sheet { rules: rules.into_iter().map(|(_, _, rule)| rule).collect() }
    }
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start + 2..].find("*/").map_or("", |end| &rest[start + 2 + end + 2..]);
    }
    out.push_str(rest);
    out
}

/// `chain` is the element and its ancestors, outermost first; `hover` says
/// whether each of them is under the mouse (all false while laying out).
fn matches(rule: &Rule, chain: &[&Element], hover: &[bool]) -> bool {
    let n = chain.len();
    let (subject, ancestors) = rule.parts.split_last().unwrap();
    if !subject.matches(chain[n - 1], hover[n - 1]) {
        return false;
    }
    let mut i = n - 1;
    for part in ancestors.iter().rev() {
        loop {
            if i == 0 {
                return false;
            }
            i -= 1;
            if part.matches(chain[i], hover[i]) {
                break;
            }
        }
    }
    true
}

// ---- layout ----

/// A laid-out box; children's positions are relative to its top left.
struct LBox<'a> {
    element: Option<&'a Element>,
    /// Wrapped lines of an anonymous text box.
    lines: Vec<String>,
    w: f32,
    h: f32,
    children: Vec<(f32, f32, LBox<'a>)>,
}

struct Ctx<'a> {
    sheet: &'a Sheet,
    measure: Measure<'a>,
    rich: Option<RichMeasure<'a>>,
}

enum Item<'a> {
    Element(&'a Element, Style),
    Text(String),
}

impl<'a> Ctx<'a> {
    fn style(&self, chain: &[&Element], parent: &Style) -> Style {
        self.style_hover(chain, &vec![false; chain.len()], parent)
    }

    fn style_hover(&self, chain: &[&Element], hover: &[bool], parent: &Style) -> Style {
        let mut style = Style::inherit(parent);
        let element = chain[chain.len() - 1];
        match element.tag.as_str() {
            "b" | "strong" => style.bold = true,
            "item" => {
                style.width = Some(Len::Px(ITEM));
                style.height = Some(ITEM);
            }
            "img" => {
                let side = |name: &str| element.attr(name).and_then(px).unwrap_or(ITEM);
                style.width = Some(Len::Px(side("width")));
                style.height = Some(side("height"));
            }
            _ => {}
        }
        for rule in &self.sheet.rules {
            if matches(rule, chain, hover) {
                for (name, value) in &rule.decls {
                    style.set(name, value);
                }
            }
        }
        if let Some(inline) = element.attr("style") {
            for (name, value) in inline.split(';').filter_map(|d| d.split_once(':')) {
                style.set(&name.trim().to_ascii_lowercase(), value.trim());
            }
        }
        style
    }

    fn items<'e>(&self, element: &'e Element, chain: &[&'e Element], style: &Style) -> Vec<Item<'e>> {
        let mut out = Vec::new();
        for child in &element.children {
            match child {
                Node::Text(text) => {
                    let mut text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if style.upper {
                        text = text.to_uppercase();
                    }
                    if !text.is_empty() {
                        out.push(Item::Text(text));
                    }
                }
                Node::Element(child) if child.tag == "br" => {}
                Node::Element(child) => {
                    let mut chain = chain.to_vec();
                    chain.push(child);
                    let s = self.style(&chain, style);
                    if !s.hidden && !s.absolute {
                        out.push(Item::Element(child, s));
                    }
                }
            }
        }
        out
    }

    /// The widest the content wants to be, padding included (max-content).
    fn max_content(&self, element: &Element, chain: &[&Element], style: &Style) -> f32 {
        if let Some(Len::Px(w)) = style.width {
            return w;
        }
        let items = self.items(element, chain, style);
        let mut sum = 0.0_f32;
        let mut max = 0.0_f32;
        for item in &items {
            let w = match item {
                Item::Text(text) => self.tw(text, style),
                Item::Element(child, s) => {
                    let mut chain = chain.to_vec();
                    chain.push(child);
                    self.max_content(child, &chain, s).max(s.min_width.unwrap_or(0.0)) + s.margin[1] + s.margin[3]
                }
            };
            sum += w;
            max = max.max(w);
        }
        let content = if style.flex && style.row { sum + style.gap * items.len().saturating_sub(1) as f32 } else { max };
        content + style.pad[1] + style.pad[3]
    }

    /// Border-box width in a container whose content box is `avail` wide.
    /// `fill`: stretch like a block (otherwise shrink to fit the content).
    fn outer_width(&self, element: &Element, chain: &[&Element], style: &Style, avail: f32, fill: bool) -> f32 {
        let room = (avail - style.margin[1] - style.margin[3]).max(0.0);
        let mut w = match style.width {
            Some(len) => len.of(avail),
            None if fill => room,
            None => self.max_content(element, chain, style).min(room),
        };
        if let Some(max) = style.max_width {
            w = w.min(max.of(avail));
        }
        let min = style.min_width.unwrap_or(0.0);
        w.max(min).min(room.max(min))
    }

    fn rich(&self, style: &Style) -> bool {
        self.rich.is_some() && !style.family.is_empty() && style.size > 0.0
    }

    fn line_height(&self, style: &Style) -> f32 {
        if self.rich(style) {
            style.line_height.unwrap_or(style.size * 1.25)
        } else {
            LINE * style.scale
        }
    }

    fn tw(&self, text: &str, style: &Style) -> f32 {
        match (&self.rich, self.rich(style)) {
            (Some(rich), true) => rich(text, &Font { family: style.family.clone(), size: style.size, weight: style.weight }),
            _ => (self.measure)(text, style.bold) * style.scale,
        }
    }

    fn wrap(&self, text: &str, width: f32, style: &Style) -> Vec<String> {
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in text.split(' ') {
            let candidate = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
            if !line.is_empty() && self.tw(&candidate, style) > width {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            } else {
                line = candidate;
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }

    fn text_box(&self, text: &str, width: f32, style: &Style) -> LBox<'a> {
        let lines = self.wrap(text, width.max(1.0), style);
        let w = lines.iter().map(|l| self.tw(l, style)).fold(0.0, f32::max);
        let h = lines.len() as f32 * self.line_height(style);
        LBox { element: None, lines, w, h, children: Vec::new() }
    }

    /// Lays out `element` at border-box width `w`.
    fn layout(&self, element: &'a Element, chain: &[&'a Element], style: &Style, w: f32) -> LBox<'a> {
        let inner_w = (w - style.pad[1] - style.pad[3]).max(0.0);
        let items = self.items(element, chain, style);
        let mut children = Vec::new();
        let content_h = if element.tag == "item" {
            0.0
        } else if style.flex && style.row {
            self.layout_row(&items, chain, style, inner_w, &mut children)
        } else {
            self.layout_column(&items, chain, style, inner_w, &mut children)
        };
        let h = style.height.unwrap_or(content_h + style.pad[0] + style.pad[2]);
        if style.height.is_some() && style.flex && style.row && style.align == Align::Center {
            // Centre the content vertically in a fixed-height row.
            let free = h - style.pad[0] - style.pad[2] - content_h;
            if free > 0.0 {
                for child in &mut children {
                    child.1 += free / 2.0;
                }
            }
        }
        for child in &mut children {
            child.0 += style.pad[3];
            child.1 += style.pad[0];
        }
        for child in &element.children {
            let Node::Element(child) = child else { continue };
            let mut chain = chain.to_vec();
            chain.push(child);
            let s = self.style(&chain, style);
            if s.hidden || !s.absolute {
                continue;
            }
            let (left, right) = (s.left.map(|l| l.of(w)), s.right.map(|l| l.of(w)));
            let (top, bottom) = (s.top.map(|l| l.of(h)), s.bottom.map(|l| l.of(h)));
            let cw = match (s.width, left, right) {
                (Some(len), ..) => len.of(w),
                (None, Some(l), Some(r)) => (w - l - r).max(0.0),
                _ => self.max_content(child, &chain, &s).min(w),
            };
            let fixed_h = s.height.or(s.height_pct.map(|p| h * p / 100.0)).or(match (top, bottom) {
                (Some(t), Some(b)) => Some((h - t - b).max(0.0)),
                _ => None,
            });
            let mut sized = s.clone();
            sized.height = fixed_h;
            let b = self.layout(child, &chain, &sized, cw);
            let x = left.or(right.map(|r| w - r - b.w)).unwrap_or(0.0);
            let y = top.or(bottom.map(|bt| h - bt - b.h)).unwrap_or(0.0);
            children.push((x, y, b));
        }
        LBox { element: Some(element), lines: Vec::new(), w, h, children }
    }

    fn layout_column(&self, items: &[Item<'a>], chain: &[&'a Element], style: &Style, inner_w: f32, out: &mut Vec<(f32, f32, LBox<'a>)>) -> f32 {
        let mut y = 0.0;
        for (n, item) in items.iter().enumerate() {
            if n > 0 && style.flex {
                y += style.gap;
            }
            match item {
                Item::Text(text) => {
                    let b = self.text_box(text, inner_w, style);
                    let x = if style.flex && style.align == Align::Center || style.text_center {
                        (inner_w - b.w) / 2.0
                    } else if style.text_right {
                        inner_w - b.w
                    } else {
                        0.0
                    };
                    let h = b.h;
                    out.push((x, y, b));
                    y += h;
                }
                Item::Element(child, s) => {
                    let mut chain = chain.to_vec();
                    chain.push(child);
                    let fill = (!style.flex || style.align == Align::Stretch) && child.tag != "item";
                    let cw = self.outer_width(child, &chain, s, inner_w, fill);
                    let b = self.layout(child, &chain, s, cw);
                    let x = match style.align {
                        Align::Center if style.flex => (inner_w - cw) / 2.0,
                        Align::End if style.flex => inner_w - cw - s.margin[1],
                        _ => s.margin[3],
                    };
                    y += s.margin[0];
                    let h = b.h;
                    out.push((x, y, b));
                    y += h + s.margin[2];
                }
            }
        }
        y
    }

    fn layout_row(&self, items: &[Item<'a>], chain: &[&'a Element], style: &Style, inner_w: f32, out: &mut Vec<(f32, f32, LBox<'a>)>) -> f32 {
        // Base sizes, then lines, then grow or shrink, then each item's layout.
        let mut base = Vec::new();
        for item in items {
            base.push(match item {
                Item::Text(text) => (self.tw(text, style).min(inner_w), 0.0, [0.0; 4]),
                Item::Element(child, s) => {
                    let mut chain = chain.to_vec();
                    chain.push(child);
                    (self.outer_width(child, &chain, s, inner_w, false), s.grow, s.margin)
                }
            });
        }
        let outer = |i: usize| base[i].0 + base[i].2[1] + base[i].2[3];
        let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
        let mut used = 0.0;
        for i in 0..items.len() {
            let line = lines.last_mut().unwrap();
            if style.wrap && !line.is_empty() && used + style.gap + outer(i) > inner_w {
                lines.push(vec![i]);
                used = outer(i);
            } else {
                if !line.is_empty() {
                    used += style.gap;
                }
                line.push(i);
                used += outer(i);
            }
        }
        let mut y = 0.0;
        for (n, line) in lines.iter().enumerate().filter(|(_, line)| !line.is_empty()) {
            if n > 0 {
                y += style.gap;
            }
            let taken: f32 = line.iter().map(|&i| outer(i)).sum::<f32>() + style.gap * (line.len() - 1) as f32;
            let mut free = inner_w - taken;
            let grow: f32 = line.iter().map(|&i| base[i].1).sum();
            let mut widths: Vec<f32> = line.iter().map(|&i| base[i].0).collect();
            if free > 0.0 && grow > 0.0 {
                for (k, &i) in line.iter().enumerate() {
                    widths[k] += free * base[i].1 / grow;
                }
                free = 0.0;
            } else if free < 0.0 {
                // Shrink what does not fit, in proportion to size. Boxes with
                // a set width keep it while others can give (browsers' min-width: auto).
                let fixed: Vec<bool> = line.iter().map(|&i| matches!(&items[i], Item::Element(_, s) if s.width.is_some())).collect();
                let flexible = |k: usize| !fixed[k] || fixed.iter().all(|&f| f);
                let total: f32 = (0..widths.len()).filter(|&k| flexible(k)).map(|k| widths[k]).sum();
                if total > 0.0 {
                    for k in 0..widths.len() {
                        if flexible(k) {
                            widths[k] = (widths[k] + free * widths[k] / total).max(0.0);
                        }
                    }
                }
                free = 0.0;
            }
            let mut boxes = Vec::new();
            let mut line_h = 0.0_f32;
            for (k, &i) in line.iter().enumerate() {
                let b = match &items[i] {
                    Item::Text(text) => self.text_box(text, widths[k], style),
                    Item::Element(child, s) => {
                        let mut chain = chain.to_vec();
                        chain.push(child);
                        self.layout(child, &chain, s, widths[k])
                    }
                };
                line_h = line_h.max(b.h + base[i].2[0] + base[i].2[2]);
                boxes.push(b);
            }
            let (mut x, spacing) = match style.justify {
                Justify::Center => (free / 2.0, style.gap),
                Justify::End => (free, style.gap),
                Justify::Between if line.len() > 1 => (0.0, style.gap + free / (line.len() - 1) as f32),
                _ => (0.0, style.gap),
            };
            for (k, mut b) in boxes.into_iter().enumerate() {
                let i = line[k];
                let m = base[i].2;
                x += m[3];
                let fixed_h = matches!(&items[i], Item::Element(_, s) if s.height.is_some());
                let dy = match style.align {
                    Align::Center => (line_h - b.h) / 2.0,
                    Align::End => line_h - b.h - m[2],
                    Align::Stretch if !fixed_h && b.element.is_some() => {
                        b.h = line_h - m[0] - m[2];
                        m[0]
                    }
                    _ => m[0],
                };
                let w = b.w;
                out.push((x, y + dy, b));
                x += w + m[1] + spacing;
            }
            y += line_h;
        }
        y
    }

    // ---- paint ----

    #[allow(clippy::too_many_arguments)]
    fn paint(&self, b: &LBox<'a>, x: f32, y: f32, chain: &mut Vec<(&'a Element, bool)>, parent: &Style, opacity: f32, mouse: (f32, f32), out: &mut Paint) {
        let Some(element) = b.element else {
            for (i, line) in b.lines.iter().enumerate() {
                let lw = self.tw(line, parent);
                let lx = if parent.text_center {
                    x + (b.w - lw) / 2.0
                } else if parent.text_right {
                    x + b.w - lw
                } else {
                    x
                };
                let rich = self.rich(parent);
                let label = Label {
                    x: lx,
                    y: y + i as f32 * self.line_height(parent) + if rich { 0.0 } else { parent.scale },
                    text: line.clone(),
                    color: fade(parent.color, opacity),
                    bold: parent.bold,
                    scale: if rich { 1.0 } else { parent.scale },
                    family: if rich { parent.family.clone() } else { String::new() },
                    size: if rich { parent.size } else { 0.0 },
                    weight: parent.weight,
                    shadow: parent.shadow,
                    line_height: if rich { self.line_height(parent) } else { 0.0 },
                };
                out.labels.push(label.clone());
                out.ops.push(Op::Label(label));
            }
            return;
        };
        let hovered = mouse.0 >= x && mouse.1 >= y && mouse.0 < x + b.w && mouse.1 < y + b.h;
        chain.push((element, hovered));
        let elements: Vec<&Element> = chain.iter().map(|c| c.0).collect();
        let hover: Vec<bool> = chain.iter().map(|c| c.1).collect();
        let style = self.style_hover(&elements, &hover, parent);
        let opacity = opacity * style.opacity;
        let fill = |out: &mut Paint, fx: f32, fy: f32, fw: f32, fh: f32, color: u32| {
            let f = Fill { x: fx, y: fy, w: fw, h: fh, color: fade(color, opacity) };
            out.fills.push(f.clone());
            out.ops.push(Op::Fill(f));
        };
        if let Some(color) = style.bg {
            fill(out, x, y, b.w, b.h, color);
        }
        if let Some((top, bottom)) = style.gradient {
            let g = Gradient { x, y, w: b.w, h: b.h, top: fade(top, opacity), bottom: fade(bottom, opacity) };
            out.gradients.push(g.clone());
            out.ops.push(Op::Gradient(g));
        }
        let image = |out: &mut Paint, src: &str| {
            let i = Image { x, y, w: b.w, h: b.h, src: src.to_owned(), opacity };
            out.images.push(i.clone());
            out.ops.push(Op::Image(i));
        };
        if let Some(src) = &style.bg_image {
            image(out, src);
        }
        if element.tag == "img" {
            if let Some(src) = element.attr("src") {
                image(out, src);
            }
        }
        if let Some(color) = style.border {
            fill(out, x, y, b.w, 1.0, color);
            fill(out, x, y + b.h - 1.0, b.w, 1.0, color);
            fill(out, x, y, 1.0, b.h, color);
            fill(out, x + b.w - 1.0, y, 1.0, b.h, color);
        }
        if let Some(onclick) = element.attr("onclick") {
            out.clicks.push(Click { x, y, w: b.w, h: b.h, onclick: onclick.to_owned() });
        }
        if element.tag == "item" {
            if let Some(id) = element.attr("id") {
                let icon = Icon { x, y, item: id.to_owned() };
                out.icons.push(icon.clone());
                out.ops.push(Op::Icon(icon));
            }
        }
        for (cx, cy, child) in &b.children {
            self.paint(child, x + cx, y + cy, chain, &style, opacity, mouse, out);
        }
        chain.pop();
    }
}

fn fade(color: u32, opacity: f32) -> u32 {
    if opacity >= 1.0 {
        return color;
    }
    let a = ((color >> 24) as f32 * opacity.clamp(0.0, 1.0)).round() as u32;
    (color & 0x00FF_FFFF) | (a << 24)
}

fn split_top_level(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0_i32, 0);
    for (i, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(text[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(text[start..].trim());
    parts
}

fn gradient(value: &str) -> Option<(u32, u32)> {
    let start = value.find("linear-gradient(")? + "linear-gradient(".len();
    let end = value.rfind(')')?;
    let args = split_top_level(&value[start..end]);
    let colors: Vec<u32> = args.iter().filter_map(|a| color(a).or_else(|| a.split_whitespace().next().and_then(color))).collect();
    let (first, last) = (*colors.first()?, *colors.last()?);
    let upward = args.first().is_some_and(|a| a.contains("to top") || a.starts_with("0deg"));
    Some(if upward { (last, first) } else { (first, last) })
}

fn url_of(value: &str) -> Option<String> {
    let start = value.find("url(")? + 4;
    let end = start + value[start..].find(')')?;
    Some(value[start..end].trim().trim_matches(|c| c == '"' || c == '\'').to_owned())
}

fn px(value: &str) -> Option<f32> {
    let value = value.trim();
    value.strip_suffix("px").unwrap_or(value).trim().parse().ok()
}

fn len(value: &str) -> Option<Len> {
    match value.trim().strip_suffix('%') {
        Some(p) => p.trim().parse().ok().map(Len::Pct),
        None => px(value).map(Len::Px),
    }
}

fn sides(value: &str) -> [f32; 4] {
    let n: Vec<f32> = value.split_whitespace().map(|v| px(v).unwrap_or(0.0)).collect();
    match n.as_slice() {
        [a, b, c, d] => [*a, *b, *c, *d],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b] => [*a, *b, *a, *b],
        [a] => [*a; 4],
        _ => [0.0; 4],
    }
}

fn color(value: &str) -> Option<u32> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let hex: String = if hex.len() == 3 { hex.chars().flat_map(|c| [c, c]).collect() } else { hex.to_owned() };
        if hex.len() != 6 {
            return None;
        }
        return u32::from_str_radix(&hex, 16).ok().map(|rgb| 0xFF00_0000 | rgb);
    }
    let inner = value.strip_prefix("rgba(").or_else(|| value.strip_prefix("rgb("))?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    let channel = |i: usize| parts.get(i)?.parse::<u32>().ok().map(|v| v.min(255));
    let (r, g, b) = (channel(0)?, channel(1)?, channel(2)?);
    let a = parts.get(3).and_then(|a| a.parse::<f32>().ok()).unwrap_or(1.0).clamp(0.0, 1.0);
    Some((((a * 255.0).round() as u32) << 24) | (r << 16) | (g << 8) | b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::document;

    fn within(x: f32, y: f32, c: &Click) -> bool {
        x >= c.x && y >= c.y && x <= c.x + c.w && y <= c.y + c.h
    }

    #[test]
    fn the_server_document_lays_out_six_clickable_cards() {
        let (html, css) = document();
        let paint = paint(html, css, 1280.0, 720.0, 0.0, 0.0);
        assert_eq!(paint.clicks.len(), 6);
        assert!(paint.clicks[0].onclick.contains("bedwars"));
        assert_eq!(paint.icons.len(), 6);
        assert!(paint.labels.iter().any(|label| label.text.contains("Выбор режима")));
    }

    #[test]
    fn every_card_holds_its_icon_and_text_inside_the_window() {
        let (html, css) = document();
        // GUI sizes: 1920x1080 at scale 3, 1280x720 at scale 3, 1280x720 at scale 1.
        for (w, h) in [(640.0, 360.0), (427.0, 240.0), (1280.0, 720.0)] {
            let paint = paint(html, css, w, h, 0.0, 0.0);
            assert_eq!(paint.windows.len(), 1);
            let (wx, wy, ww, wh) = paint.windows[0];
            assert!(ww <= w && wx >= 0.0, "{w}: window {ww} wide");
            for c in &paint.clicks {
                assert!(c.x >= wx && c.y >= wy && c.x + c.w <= wx + ww + 0.01 && c.y + c.h <= wy + wh + 0.01, "{w}: card outside the window");
            }
            for icon in &paint.icons {
                assert!(paint.clicks.iter().any(|c| within(icon.x, icon.y, c) && within(icon.x + ITEM, icon.y + ITEM, c)), "{w}: icon outside a card");
            }
            for name in ["Bed Wars", "Прятки", "СКОРО"] {
                let label = paint.labels.iter().find(|l| l.text == name).unwrap();
                assert!(paint.clicks.iter().any(|c| within(label.x, label.y, c) && within(label.x, label.y + LINE, c)), "{w}: {name} outside a card");
            }
            for (i, a) in paint.clicks.iter().enumerate() {
                for b in &paint.clicks[i + 1..] {
                    let overlap = a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
                    assert!(!overlap, "{w}: cards overlap");
                }
            }
        }
    }

    #[test]
    fn three_cards_per_row_on_a_wide_screen() {
        let (html, css) = document();
        let paint = paint(html, css, 640.0, 360.0, 0.0, 0.0);
        let first_row = paint.clicks.iter().filter(|c| c.y == paint.clicks[0].y).count();
        assert_eq!(first_row, 3);
    }

    #[test]
    fn hover_changes_the_card_border() {
        let (html, css) = document();
        let idle = paint(html, css, 640.0, 360.0, -1.0, -1.0);
        let card = idle.clicks[0].clone();
        let hot = paint(html, css, 640.0, 360.0, card.x + 5.0, card.y + 5.0);
        let gold = 0xFFEB_CE96;
        let edges = |p: &Paint| p.fills.iter().filter(|f| f.color == gold && f.x >= card.x && f.y >= card.y && f.x < card.x + card.w && f.y < card.y + card.h).count();
        assert_eq!(edges(&hot), edges(&idle) + 4);
    }

    #[test]
    fn descendant_and_hover_selectors() {
        let html = "<div class=a><div class=b>x</div></div><div class=b>y</div>";
        let css = ".b { color: #111111 } .a .b { color: #222222 } .a:hover .b { color: #333333 }";
        let p = paint(html, css, 200.0, 100.0, -1.0, -1.0);
        assert_eq!(p.labels[0].color, 0xFF22_2222);
        assert_eq!(p.labels[1].color, 0xFF11_1111);
        let (x, y, _, _) = p.windows[0];
        let p = paint(html, css, 200.0, 100.0, x + 1.0, y + 1.0);
        assert_eq!(p.labels[0].color, 0xFF33_3333);
    }

    #[test]
    fn a_box_with_a_set_width_does_not_shrink_next_to_long_text() {
        let html = "<div class=row><div class=icon></div><div>очень длинный текст который не помещается в одну строку</div></div>";
        let css = ".row { display: flex; width: 120px; gap: 4px } .icon { width: 28px; height: 28px; background: #101010 }";
        let p = paint(html, css, 300.0, 200.0, 0.0, 0.0);
        let icon = p.fills.iter().find(|f| f.color == 0xFF10_1010).unwrap();
        assert_eq!(icon.w, 28.0);
        assert!(p.labels.iter().all(|l| l.x >= icon.x + 28.0));
    }

    fn scene(html: &str, css: &str) -> Paint {
        let html = format!("<div id=scene>{html}</div>");
        let css = format!("#scene {{ width: 400px; height: 200px }} {css}");
        paint(&html, &css, 400.0, 200.0, -1.0, -1.0)
    }

    #[test]
    fn absolute_boxes_sit_at_their_edges_and_leave_the_flow() {
        let p = scene(
            "<div class=a></div><div class=b></div><div class=c></div><div class=flow>x</div>",
            ".a { position: absolute; left: 10px; top: 20px; width: 30px; height: 40px; background: #010101 } \
             .b { position: absolute; right: 5px; bottom: 7px; width: 50px; height: 60px; background: #020202 } \
             .c { position: absolute; left: 10%; top: 50%; width: 20%; height: 25%; background: #030303 }",
        );
        let find = |c: u32| p.fills.iter().find(|f| f.color == c).unwrap().clone();
        let (a, b, c) = (find(0xFF01_0101), find(0xFF02_0202), find(0xFF03_0303));
        assert_eq!((a.x, a.y, a.w, a.h), (0.0 + 10.0, 0.0 + 20.0, 30.0, 40.0));
        assert_eq!((b.x, b.y), (400.0 - 5.0 - 50.0, 200.0 - 7.0 - 60.0));
        assert_eq!((c.x, c.y, c.w, c.h), (40.0, 100.0, 80.0, 50.0));
        let label = &p.labels[0];
        assert_eq!((label.x, label.y), (0.0, 1.0));
    }

    #[test]
    fn images_come_from_img_tags_and_background_urls_in_paint_order() {
        let p = scene(
            "<div class=card><img src=\"dota/a\" width=24 height=12></div>",
            ".card { width: 64px; height: 32px; background: #101010; background-image: url(\"dota/frame\") }",
        );
        assert_eq!(p.images.len(), 2);
        assert_eq!(p.images[0].src, "dota/frame");
        assert_eq!((p.images[0].w, p.images[0].h), (64.0, 32.0));
        assert_eq!(p.images[1].src, "dota/a");
        assert_eq!((p.images[1].w, p.images[1].h), (24.0, 12.0));
        let kinds: Vec<&str> = p
            .ops
            .iter()
            .map(|o| match o {
                Op::Fill(_) => "fill",
                Op::Image(_) => "image",
                Op::Gradient(_) => "gradient",
                Op::Label(_) => "label",
                Op::Icon(_) => "icon",
            })
            .collect();
        assert_eq!(kinds, ["fill", "image", "image"]);
    }

    #[test]
    fn gradients_take_the_outer_colors_and_can_run_upward() {
        let p = scene(
            "<div class=a></div><div class=b></div>",
            ".a { width: 10px; height: 10px; background: linear-gradient(to bottom, #ff0000, rgba(0, 0, 255, 0.5)) } \
             .b { width: 10px; height: 10px; background-image: linear-gradient(to top, #00ff00, #0000ff) }",
        );
        assert_eq!((p.gradients[0].top, p.gradients[0].bottom), (0xFFFF_0000, 0x800000FF));
        assert_eq!((p.gradients[1].top, p.gradients[1].bottom), (0xFF00_00FF, 0xFF00_FF00));
    }

    #[test]
    fn opacity_multiplies_down_the_tree() {
        let p = scene(
            "<div class=a><div class=b>hi</div></div>",
            ".a { opacity: 0.5; background: #ffffff; width: 20px; height: 20px } .b { opacity: 0.5; background: #ffffff; width: 10px; height: 10px; color: #ffffff }",
        );
        assert_eq!(p.fills[0].color >> 24, 128);
        assert_eq!(p.fills[1].color >> 24, 64);
        assert_eq!(p.labels[0].color >> 24, 64);
    }

    #[test]
    fn font_size_scales_text_and_its_box() {
        let p = scene(
            "<div class=small>abcd</div><div class=big>abcd</div>",
            ".small { width: 100px } .big { font-size: 16px; width: 100px }",
        );
        assert_eq!(p.labels[0].scale, 1.0);
        assert_eq!(p.labels[1].scale, 2.0);
        assert!(p.labels[1].y > p.labels[0].y + LINE);
        let tight = scene("<div class=big>abcd efgh</div>", ".big { font-size: 24px; width: 60px }");
        assert!(tight.labels.len() > 1, "scaled text wraps by its scaled width");
    }

    #[test]
    fn text_aligns_right_inside_its_box() {
        let p = scene(
            "<div class=r>ab</div><div class=c>ab</div>",
            ".r { width: 100px; text-align: right } .c { width: 100px; text-align: center }",
        );
        let width = 12.0;
        assert_eq!(p.labels[0].x, 100.0 - width);
        assert_eq!(p.labels[1].x, (100.0 - width) / 2.0);
    }

    #[test]
    fn real_fonts_carry_family_size_weight_shadow_and_case() {
        let html = "<div id=scene><div class=t>hello world</div></div>";
        let css = "#scene { width: 400px; height: 200px } .t { font-family: Radiance, Arial; font-size: 20px; font-weight: 600; text-transform: uppercase; text-shadow: 1px 1px #000000; line-height: 30px; width: 400px }";
        let rich = |text: &str, font: &Font| text.chars().count() as f32 * font.size * 0.5;
        let legacy = |text: &str, _bold: bool| text.chars().count() as f32 * 6.0;
        let p = paint_rich(html, css, 400.0, 200.0, -1.0, -1.0, &legacy, Some(&rich));
        let l = &p.labels[0];
        assert_eq!(l.text, "HELLO WORLD");
        assert_eq!(l.family, "Radiance");
        assert_eq!(l.size, 20.0);
        assert_eq!(l.weight, 600);
        assert_eq!(l.shadow, Some(0xFF00_0000));
        assert_eq!(l.scale, 1.0);
    }

    #[test]
    fn the_rich_width_wraps_text_and_sets_the_line_height() {
        let html = "<div id=scene><div class=t>aaaa bbbb cccc</div></div>";
        let css = "#scene { width: 400px; height: 200px } .t { font-family: Radiance; font-size: 10px; width: 40px; line-height: 14px }";
        let rich = |text: &str, font: &Font| text.chars().count() as f32 * font.size * 0.6;
        let legacy = |text: &str, _bold: bool| text.chars().count() as f32 * 6.0;
        let p = paint_rich(html, css, 400.0, 200.0, -1.0, -1.0, &legacy, Some(&rich));
        assert_eq!(p.labels.len(), 3);
        assert_eq!(p.labels[1].y - p.labels[0].y, 14.0);
    }

    #[test]
    fn without_a_rich_measure_the_old_font_path_is_unchanged() {
        let html = "<div id=scene><div class=t>abc</div></div>";
        let css = "#scene { width: 100px; height: 100px } .t { font-family: Radiance; font-size: 16px }";
        let p = paint(html, css, 100.0, 100.0, -1.0, -1.0);
        assert_eq!(p.labels[0].scale, 2.0);
        assert_eq!(p.labels[0].family, "");
    }

    #[test]
    fn long_text_wraps_inside_its_box() {
        let p = paint("<div class=box>один два три четыре пять шесть семь</div>", ".box { width: 60px }", 300.0, 200.0, 0.0, 0.0);
        assert!(p.labels.len() > 1);
        assert!(p.labels.iter().all(|l| l.text.chars().count() as f32 * 6.0 <= 60.0 || !l.text.contains(' ')));
    }
}
