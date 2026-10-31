use crate::wire::Snapshot;
use std::fmt::Write;

pub const TEXTURES: &[&str] = &[
    "overview",
    "reborn/bg_minimap",
    "reborn/ability_bg",
    "reborn/inventory_bg",
    "reborn/ability_bevel",
    "reborn/active_ability_border",
    "reborn/ability_inset_shadow",
    "reborn/gold_large",
    "passive_ability_border",
    "learnspell",
    "hudv2_portrait_levelup_2",
];

const REFERENCE_HEIGHT: f32 = 1080.0;
const PANEL_H: f32 = 145.0;
const STATS_W: f32 = 159.0;
const PORTRAIT_W: f32 = 159.0;
const CENTER_W: f32 = 570.0;
const INVENTORY_W: f32 = 270.0;
const ABILITY_W: f32 = 72.0;
const ABILITY_ICON: f32 = 64.0;
const HEALTH_H: f32 = 24.0;
const MANA_H: f32 = 18.0;
const SLOT_W: f32 = 65.0;
const SLOT_H: f32 = 50.0;

pub fn pretty_hero(name: &str) -> String {
    let base = name.trim_start_matches("npc_dota_hero_").replace('_', " ");
    let mut out = String::new();
    let mut up = true;
    for c in base.chars() {
        if up {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        up = c == ' ';
    }
    out
}

pub fn short_name(name: &str, chars: usize) -> String {
    let stripped = name.strip_prefix("item_");
    let words: Vec<&str> = match stripped {
        Some(rest) => rest.split('_').collect(),
        None => name.split('_').skip(1).collect(),
    };
    words.join(" ").chars().take(chars).collect()
}

fn seconds(value: f32) -> String {
    if value >= 10.0 {
        format!("{}", value.ceil() as i32)
    } else {
        format!("{value:.1}")
    }
}

struct Doc {
    k: f32,
    html: String,
    css: String,
    next: u32,
}

impl Doc {
    fn px(&self, v: f32) -> String {
        format!("{:.1}px", v * self.k)
    }

    fn font(&self, dota_px: f32) -> String {
        format!("{:.1}px", (dota_px * self.k).max(4.0))
    }

    fn rule(&mut self, class: &str, body: &str) {
        let _ = write!(self.css, ".{class}{{{body}}}");
    }

    fn open(&mut self, class: &str, id: Option<&str>) {
        match id {
            Some(id) => {
                let _ = write!(self.html, "<div class=\"{class}\" id=\"{id}\">");
            }
            None => {
                let _ = write!(self.html, "<div class=\"{class}\">");
            }
        }
    }

    fn close(&mut self) {
        self.html.push_str("</div>");
    }

    fn at(&mut self, left: f32, top: f32, w: f32, h: f32, extra: &str, inner: &str) {
        self.next += 1;
        let class = format!("n{}", self.next);
        let body = format!(
            "position:absolute;left:{};top:{};width:{};height:{};{extra}",
            self.px(left),
            self.px(top),
            self.px(w),
            self.px(h),
        );
        self.rule(&class, &body);
        self.open(&class, None);
        self.html.push_str(inner);
        self.close();
    }

    fn text(&mut self, left: f32, top: f32, w: f32, h: f32, size: f32, color: &str, centered: bool, text: &str) {
        self.text_f("Radiance", 400, left, top, w, h, size, color, centered, text);
    }

    fn text_f(&mut self, family: &str, weight: u16, left: f32, top: f32, w: f32, h: f32, size: f32, color: &str, centered: bool, text: &str) {
        let align = if centered { "text-align:center;" } else { "" };
        let extra = format!("font-family:{family};font-weight:{weight};font-size:{};color:{color};{align}", self.font(size));
        self.at(left, top, w, h, &extra, &escape(text));
    }

    fn number(&mut self, left: f32, top: f32, w: f32, h: f32, size: f32, color: &str, centered: bool, text: &str) {
        self.text_f("RadianceM", 500, left, top, w, h, size, color, centered, text);
    }

    fn vgradient(&mut self, left: f32, top: f32, w: f32, h: f32, stops: &[(f32, &str)]) {
        for pair in stops.windows(2) {
            let (a, ca) = pair[0];
            let (b, cb) = pair[1];
            let extra = format!("background:linear-gradient(to bottom,{ca},{cb});");
            self.at(left, top + h * a, w, h * (b - a), &extra, "");
        }
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

const HEALTH_STOPS: [(f32, &str); 4] = [(0.0, "#425d25"), (0.2, "#5BA539"), (0.5, "#4D9030"), (1.0, "#425d25")];
const HEALTH_BACK: [(f32, &str); 4] = [(0.0, "#17200d"), (0.2, "#222e13"), (0.5, "#1e2911"), (1.0, "#17200d")];
const MANA_STOPS: [(f32, &str); 4] = [(0.0, "#2b4287"), (0.2, "#4165ce"), (0.5, "#4a73ea"), (1.0, "#2b4287")];
const MANA_BACK: [(f32, &str); 4] = [(0.0, "#101932"), (0.2, "#172447"), (0.5, "#162244"), (1.0, "#101932")];

fn bar(doc: &mut Doc, left: f32, top: f32, w: f32, h: f32, fraction: f32, fill: &[(f32, &str)], back: &[(f32, &str)], label: &str, regen: &str) {
    doc.vgradient(left, top, w, h, back);
    let filled = (w * fraction.clamp(0.0, 1.0)).max(0.0);
    if filled > 0.5 {
        doc.vgradient(left, top, filled, h, fill);
    }
    doc.number(left, top + h * 0.5 - 8.0, w, h, 15.0, "#ffffff", true, label);
    if !regen.is_empty() {
        doc.at(
            left + w - 70.0,
            top + h * 0.5 - 6.0,
            66.0,
            14.0,
            &format!("font-family:RadianceM;font-weight:500;font-size:{};color:#b9d8b5;text-align:right;", doc.font(13.0)),
            &escape(regen),
        );
    }
}

pub fn document_overlay(s: &Snapshot, width: f32, height: f32, has_icon: &dyn Fn(&str) -> bool) -> (String, String) {
    let k = height / REFERENCE_HEIGHT;
    let mut doc = Doc {
        k,
        html: String::new(),
        css: String::new(),
        next: 0,
    };
    doc.css.push_str("#scene{position:relative;font-family:Radiance;font-weight:400;text-shadow:1px 1px #000000;}");
    doc.html.push_str("<div id=\"scene\" style=\"width:");
    let _ = write!(doc.html, "{width:.1}px;height:{height:.1}px\">");
    banner(&mut doc, s, width / k, height / k);
    doc.html.push_str("</div>");
    (doc.html, doc.css)
}

pub fn document(s: &Snapshot, width: f32, height: f32, has_icon: &dyn Fn(&str) -> bool) -> (String, String) {
    let k = height / REFERENCE_HEIGHT;
    let mut doc = Doc {
        k,
        html: String::new(),
        css: String::new(),
        next: 0,
    };
    doc.css.push_str("#scene{position:relative;font-family:Radiance;font-weight:400;text-shadow:1px 1px #000000;}");
    doc.html.push_str("<div id=\"scene\" style=\"width:");
    let _ = write!(doc.html, "{width:.1}px;height:{height:.1}px\">");

    let total = STATS_W + PORTRAIT_W + CENTER_W + INVENTORY_W;
    let x0 = (width / k - total) / 2.0;
    let top = height / k - PANEL_H;
    let stats_x = x0;
    let portrait_x = x0 + STATS_W;
    let center_x = portrait_x + PORTRAIT_W;
    let inventory_x = center_x + CENTER_W;

    top_bar(&mut doc, s, width / k, has_icon);
    minimap(&mut doc, s, height / k);
    banner(&mut doc, s, width / k, height / k);

    doc.at(
        center_x,
        top,
        CENTER_W,
        PANEL_H,
        "background-image:url(dota/hud/reborn/ability_bg);background-color:#1620346c;",
        "",
    );

    stats_panel(&mut doc, s, stats_x, top);
    portrait_panel(&mut doc, s, portrait_x, top, has_icon);
    abilities(&mut doc, s, center_x, top, has_icon);

    let bar_left = center_x + 18.0;
    let bar_w = CENTER_W - 36.0;
    let hp_top = top + 92.0;
    bar(
        &mut doc,
        bar_left,
        hp_top,
        bar_w,
        HEALTH_H,
        s.hp / s.max_hp.max(1.0),
        &HEALTH_STOPS,
        &HEALTH_BACK,
        &format!("{:.0} / {:.0}", s.hp, s.max_hp),
        &format!("+{:.1}", s.hp_regen),
    );
    bar(
        &mut doc,
        bar_left,
        hp_top + HEALTH_H + 3.0,
        bar_w,
        MANA_H,
        s.mana / s.max_mana.max(1.0),
        &MANA_STOPS,
        &MANA_BACK,
        &format!("{:.0} / {:.0}", s.mana, s.max_mana),
        &format!("+{:.1}", s.mana_regen),
    );

    inventory_panel(&mut doc, s, inventory_x, top, has_icon);

    if !s.alive {
        doc.at(0.0, 0.0, width / k, top, "background:linear-gradient(to bottom,rgba(60,0,0,0.45),rgba(30,0,0,0.45));", "");
        doc.text(0.0, top / 2.0 - 20.0, width / k, 40.0, 40.0, "#ff6a6a", true, &format!("Respawn in {}", seconds(s.respawn_in)));
    }

    doc.html.push_str("</div>");
    (doc.html, doc.css)
}

const MINIMAP_BLOCK: f32 = 244.0;
const MINIMAP_INSET: f32 = 8.0;

fn minimap(doc: &mut Doc, s: &Snapshot, height: f32) {
    let top = height - MINIMAP_BLOCK;
    let side = MINIMAP_BLOCK - MINIMAP_INSET * 2.0;
    doc.at(0.0, top, MINIMAP_BLOCK, MINIMAP_BLOCK, "background:#05070a;", "");
    doc.at(0.0, top, MINIMAP_BLOCK, MINIMAP_BLOCK, "background-image:url(dota/hud/reborn/bg_minimap);", "");
    doc.at(MINIMAP_INSET, top + MINIMAP_INSET, side, side, "background-image:url(dota/hud/overview);", "");
    let mut order: Vec<&crate::wire::Marker> = s.markers.iter().collect();
    order.sort_by_key(|m| match m.kind {
        crate::wire::MARKER_CREEP | crate::wire::MARKER_SIEGE => 1,
        crate::wire::MARKER_HERO => 3,
        _ => 0,
    });
    for m in order {
        let [u, v] = crate::map::minimap_uv([m.x, 0.0, m.z]);
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            continue;
        }
        let size = match m.kind {
            crate::wire::MARKER_HERO => 12.0,
            crate::wire::MARKER_CREEP => 5.0,
            crate::wire::MARKER_SIEGE => 7.0,
            crate::wire::MARKER_NEUTRAL => 11.0,
            crate::wire::MARKER_COURIER => 8.0,
            crate::wire::MARKER_RUNE => 9.0,
            crate::wire::MARKER_TOWER => 9.0,
            crate::wire::MARKER_BARRACKS => 8.0,
            _ => 15.0,
        };
        let color = if m.kind == crate::wire::MARKER_NEUTRAL {
            "#ffd84a"
        } else if m.kind == crate::wire::MARKER_COURIER {
            "#40e0d0"
        } else if m.kind == crate::wire::MARKER_RUNE {
            "#ff9be8"
        } else if m.radiant {
            "#4cd964"
        } else {
            "#ff4d4d"
        };
        let edge = if m.you { "border:#ffffff;" } else { "" };
        doc.at(
            MINIMAP_INSET + u * side - size / 2.0,
            top + MINIMAP_INSET + v * side - size / 2.0,
            size,
            size,
            &format!("background:{color};{edge}"),
            "",
        );
    }
}

fn banner(doc: &mut Doc, s: &Snapshot, width: f32, height: f32) {
    if s.winner == 0 {
        return;
    }
    let (text, color) = if s.winner == 1 { ("RADIANT VICTORY", "#7dff8a") } else { ("DIRE VICTORY", "#ff7a7a") };
    doc.at(0.0, height * 0.28, width, 110.0, "background:linear-gradient(to bottom,rgba(0,0,0,0.0),rgba(0,0,0,0.75));", "");
    doc.text_f("Reaver", 700, 0.0, height * 0.28 + 24.0, width, 70.0, 56.0, color, true, text);
}

fn top_bar(doc: &mut Doc, s: &Snapshot, width: f32, has_icon: &dyn Fn(&str) -> bool) {
    let cell = 112.0;
    let (mut left, mut right) = (width / 2.0 - 60.0, width / 2.0 + 60.0);
    for t in &s.team {
        let (x, back, edge) = if t.radiant {
            left -= cell + 4.0;
            (left, "#1f3a22", "#4cae50")
        } else {
            let x = right + 4.0;
            right += cell + 4.0;
            (x, "#3d1c1c", "#c24a4a")
        };
        doc.at(x, 0.0, cell, 46.0, &format!("background:{back};border:{edge};"), "");
        let portrait = format!("dota/icons/hero/{}", t.hero);
        let mut label_x = x + 4.0;
        if has_icon(&portrait) {
            let dim = if t.alive { "" } else { "opacity:0.4;" };
            doc.at(x + 3.0, 3.0, 40.0, 22.0, &format!("background-image:url({portrait});{dim}"), "");
            label_x = x + 46.0;
        }
        doc.text(label_x, 4.0, cell - (label_x - x) - 4.0, 20.0, 13.0, "#ffffff", false, &pretty_hero(&t.hero));
        let kda = format!("{}  {}/{}/{}", t.level, t.kills, t.deaths, t.assists);
        doc.text(x + 4.0, 22.0, cell - 8.0, 16.0, 13.0, "#e6d9a6", false, &kda);
        if t.alive {
            doc.at(x + 3.0, 40.0, (cell - 6.0) * t.hp_fraction.clamp(0.0, 1.0), 4.0, "background:#5BA539;", "");
        } else {
            doc.text(x + 4.0, 38.0, cell - 8.0, 10.0, 11.0, "#ff8a8a", false, &format!("{}s", seconds(t.respawn_in)));
        }
    }
    doc.at(width / 2.0 - 48.0, 0.0, 96.0, 36.0, "background:#101018;border:#5b4a2e;", "");
    doc.text(width / 2.0 - 48.0, 9.0, 96.0, 20.0, 18.0, "#e6d9a6", true, &format!("{}:{:02}", s.tick / 30 / 60, s.tick / 30 % 60));
}

fn stats_panel(doc: &mut Doc, s: &Snapshot, x: f32, top: f32) {
    doc.at(x, top, STATS_W, PANEL_H, "background:#0e0f14d8;border:#3a3326;", "");
    let rows = [
        ("DMG", format!("{:.0}-{:.0}", s.damage_min, s.damage_max)),
        ("ASPD", format!("{:.0}", s.attack_speed)),
        ("ARM", format!("{:.1}", s.armor)),
        ("MRES", format!("{:.0}%", s.magic_resist)),
        ("MS", format!("{:.0}", s.move_speed)),
    ];
    for (i, (name, value)) in rows.iter().enumerate() {
        let y = top + 10.0 + i as f32 * 19.0;
        doc.text(x + 8.0, y, 70.0, 18.0, 11.0, "#9a9486", false, name);
        doc.at(
            x + 70.0,
            y,
            STATS_W - 78.0,
            18.0,
            &format!("font-family:RadianceM;font-weight:500;font-size:{};color:#f0e6c8;text-align:right;", doc.font(14.0)),
            &escape(value),
        );
    }
    let attributes = [
        ("STR", s.strength, "#ec3d06"),
        ("AGI", s.agility, "#26e030"),
        ("INT", s.intelligence, "#00d9ec"),
    ];
    for (i, (name, value, color)) in attributes.iter().enumerate() {
        let ax = x + 8.0 + i as f32 * 50.0;
        doc.text(ax, top + 108.0, 46.0, 14.0, 10.0, color, false, name);
        doc.number(ax, top + 121.0, 46.0, 18.0, 14.0, "#f0e6c8", false, &format!("{value:.0}"));
    }
}

fn portrait_panel(doc: &mut Doc, s: &Snapshot, x: f32, top: f32, has_icon: &dyn Fn(&str) -> bool) {
    doc.at(
        x,
        top,
        PORTRAIT_W,
        PANEL_H,
        "background-image:url(dota/hud/reborn/inventory_bg);background-color:#0b0b10;",
        "",
    );
    doc.at(
        x + 8.0,
        top + 8.0,
        PORTRAIT_W - 16.0,
        PANEL_H - 40.0,
        "background:linear-gradient(to bottom,#26303f,#0a0d14);",
        "",
    );
    let portrait = format!("dota/icons/hero/{}", s.hero);
    if has_icon(&portrait) {
        doc.at(x + 8.0, top + 12.0, PORTRAIT_W - 16.0, (PORTRAIT_W - 16.0) * 72.0 / 128.0, &format!("background-image:url({portrait});"), "");
        doc.text_f("Reaver", 600, x + 8.0, top + 12.0 + (PORTRAIT_W - 16.0) * 72.0 / 128.0 + 4.0, PORTRAIT_W - 16.0, 22.0, 18.0, "#f0e6c8", true, &pretty_hero(&s.hero));
    } else {
        doc.text(x + 8.0, top + 58.0, PORTRAIT_W - 16.0, 28.0, 22.0, "#f0e6c8", true, &pretty_hero(&s.hero));
    }
    doc.at(x + 10.0, top + PANEL_H - 30.0, 36.0, 36.0, "background:#0a0a0f;border:#8a7340;", "");
    doc.number(x + 10.0, top + PANEL_H - 25.0, 36.0, 22.0, 22.0, "#ffffff", true, &s.level.to_string());
    let span = (s.xp_next_level - s.xp_this_level).max(1) as f32;
    let progress = ((s.xp - s.xp_this_level) as f32 / span).clamp(0.0, 1.0);
    doc.at(x + 52.0, top + PANEL_H - 18.0, PORTRAIT_W - 62.0, 8.0, "background:#1b1030;", "");
    if progress > 0.0 {
        doc.at(
            x + 52.0,
            top + PANEL_H - 18.0,
            (PORTRAIT_W - 62.0) * progress,
            8.0,
            "background:linear-gradient(to bottom,#c8a2ff,#7a4dc2);",
            "",
        );
    }
    if s.skill_points > 0 {
        doc.at(
            x + 8.0,
            top - 40.0,
            139.0,
            34.0,
            "background-image:url(dota/hud/hudv2_portrait_levelup_2);",
            "",
        );
        doc.text(x + 8.0, top - 32.0, 139.0, 20.0, 16.0, "#ffe27a", true, "LEVEL UP");
    }
}

fn abilities(doc: &mut Doc, s: &Snapshot, center_x: f32, top: f32, has_icon: &dyn Fn(&str) -> bool) {
    let count = s.abilities.len() as f32;
    let row_w = count * ABILITY_W;
    let left = center_x + (CENTER_W - row_w) / 2.0;
    let keys = ["Q", "E", "R", "F", "G", "H"];
    for (i, a) in s.abilities.iter().enumerate() {
        let x = left + i as f32 * ABILITY_W + (ABILITY_W - ABILITY_ICON) / 2.0;
        let y = top + 12.0;
        let learned = a.level > 0;
        let back = if !learned {
            "#1a1b20"
        } else if a.passive {
            "#1f3552"
        } else {
            "#243b2a"
        };
        doc.at(x, y, ABILITY_ICON, ABILITY_ICON, &format!("background:{back};"), "");
        let icon = format!("dota/icons/spell/{}", a.name);
        if has_icon(&icon) {
            let dim = if learned { "" } else { "opacity:0.45;" };
            doc.at(x, y, ABILITY_ICON, ABILITY_ICON, &format!("background-image:url({icon});{dim}"), "");
        } else {
            doc.text(x, y + 22.0, ABILITY_ICON, 18.0, 13.0, if learned { "#e8e2cc" } else { "#6a6a70" }, true, &short_name(&a.name, 9));
        }
        let border = if a.passive {
            "dota/hud/passive_ability_border"
        } else {
            "dota/hud/reborn/active_ability_border"
        };
        let faded = if learned { "" } else { "opacity:0.3;" };
        doc.at(x - 3.0, y - 3.0, ABILITY_ICON + 6.0, ABILITY_ICON + 6.0, &format!("background-image:url({border});{faded}"), "");
        doc.at(x, y, ABILITY_ICON, ABILITY_ICON, "background-image:url(dota/hud/reborn/ability_inset_shadow);opacity:0.6;", "");
        if a.can_level {
            doc.at(x - 4.0, y - 4.0, ABILITY_ICON + 8.0, ABILITY_ICON + 8.0, "background-image:url(dota/hud/learnspell);", "");
        }
        if learned && a.cooldown > 0.0 && a.cooldown_total > 0.0 {
            let f = (a.cooldown / a.cooldown_total).clamp(0.0, 1.0);
            doc.at(x, y, ABILITY_ICON, ABILITY_ICON * f, "background:rgba(0,0,0,0.7);", "");
            doc.text(x, y + 20.0, ABILITY_ICON, 26.0, 24.0, "#ffffff", true, &seconds(a.cooldown));
        }
        for pip in 0..a.max_level {
            let lit = if pip < a.level { "#ffd84a" } else { "#4a4130" };
            doc.at(x + 6.0 + pip as f32 * 11.0, y + ABILITY_ICON - 7.0, 9.0, 4.0, &format!("background:{lit};"), "");
        }
        if let Some(key) = keys.get(i) {
            doc.text(x + 3.0, y + 2.0, 20.0, 16.0, 13.0, "#ffe27a", false, key);
        }
        if a.mana > 0.0 && !a.passive {
            doc.text(x, y + ABILITY_ICON + 2.0, ABILITY_ICON, 14.0, 12.0, "#6f9bff", true, &format!("{:.0}", a.mana));
        }
    }
}

fn inventory_panel(doc: &mut Doc, s: &Snapshot, x: f32, top: f32, has_icon: &dyn Fn(&str) -> bool) {
    doc.at(
        x,
        top,
        INVENTORY_W,
        PANEL_H,
        "background-image:url(dota/hud/reborn/inventory_bg);background-color:#0b0b10;",
        "",
    );
    for i in 0..6usize {
        let col = (i % 3) as f32;
        let row = (i / 3) as f32;
        let sx = x + 36.0 + col * (SLOT_W + 3.0);
        let sy = top + 12.0 + row * (SLOT_H + 3.0);
        doc.at(sx, sy, SLOT_W, SLOT_H, "background:#14151a;border:#4a4130;", "");
        if let Some(Some(item)) = s.items.get(i) {
            let icon = format!("dota/icons/item/{}", item.name);
            if has_icon(&icon) {
                doc.at(sx, sy, SLOT_W, SLOT_H, &format!("background-image:url({icon});"), "");
            } else {
                doc.text(sx, sy + 17.0, SLOT_W, 18.0, 13.0, "#ece6cf", true, &short_name(&item.name, 9));
            }
            if item.charges > 1 {
                doc.text(sx + 4.0, sy + SLOT_H - 16.0, 30.0, 14.0, 13.0, "#ffd84a", false, &item.charges.to_string());
            }
            if item.cooldown > 0.0 {
                doc.at(sx, sy, SLOT_W, SLOT_H, "background:rgba(0,0,0,0.65);", "");
                doc.text(sx, sy + 12.0, SLOT_W, 26.0, 22.0, "#ffffff", true, &seconds(item.cooldown));
            }
        }
        doc.text(sx + 3.0, sy + 2.0, 16.0, 14.0, 12.0, "#8a8576", false, &(i + 1).to_string());
    }
    doc.at(x + 36.0, top + PANEL_H - 30.0, 24.0, 24.0, "background-image:url(dota/hud/reborn/gold_large);", "");
    doc.number(x + 64.0, top + PANEL_H - 29.0, 120.0, 22.0, 22.0, "#ffd84a", false, &s.gold.to_string());
}

#[derive(Clone, Debug, Default)]
pub struct ShopState {
    pub tab: usize,
    pub hover: Option<String>,
}

fn pretty_words(name: &str) -> String {
    name.split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Doc {
    fn click(&mut self, left: f32, top: f32, w: f32, h: f32, extra: &str, hover: &str, inner: &str, onclick: &str) {
        self.next += 1;
        let class = format!("n{}", self.next);
        let body = format!(
            "position:absolute;left:{};top:{};width:{};height:{};{extra}",
            self.px(left),
            self.px(top),
            self.px(w),
            self.px(h),
        );
        self.rule(&class, &body);
        if !hover.is_empty() {
            self.rule(&format!("{class}:hover"), hover);
        }
        let _ = write!(self.html, "<div class=\"{class}\" onclick=\"{onclick}\">");
        self.html.push_str(inner);
        self.close();
    }
}

pub const SHOP_COLUMNS: usize = 6;

pub fn visible_shops(data: &crate::data::Data) -> Vec<&crate::data::Shop> {
    const HIDDEN: [&str; 6] = ["pregame", "sideshop", "diretide", "nemestice", "winter", "event"];
    data.shops.iter().filter(|shop| !HIDDEN.iter().any(|h| shop.name.contains(h))).collect()
}

pub fn shop_document(
    s: &Snapshot,
    data: &crate::data::Data,
    state: &ShopState,
    width: f32,
    height: f32,
    has_icon: &dyn Fn(&str) -> bool,
) -> (String, String) {
    let k = height / REFERENCE_HEIGHT;
    let mut doc = Doc {
        k,
        html: String::new(),
        css: String::new(),
        next: 5000,
    };
    doc.css.push_str("#scene{position:relative;font-family:Radiance;font-weight:400;text-shadow:1px 1px #000000;}");
    let _ = write!(doc.html, "<div id=\"scene\" style=\"width:{width:.1}px;height:{height:.1}px\">");

    let (win_w, win_h) = (980.0, 600.0);
    let x = (width / k - win_w) / 2.0;
    let y = height / k - PANEL_H - win_h - 14.0;
    doc.at(x, y, win_w, win_h, "background:#0b0c11f2;border:#6b5a35;", "");
    doc.text_f("Reaver", 700, x + 16.0, y + 10.0, 200.0, 28.0, 26.0, "#e6d9a6", false, "SHOP");
    doc.at(
        x + win_w - 220.0,
        y + 10.0,
        200.0,
        26.0,
        &format!("font-family:RadianceM;font-weight:500;font-size:{};color:#ffd84a;text-align:right;", doc.font(24.0)),
        &format!("{} gold", s.gold),
    );
    if !s.at_shop {
        doc.text(x + 120.0, y + 12.0, win_w - 360.0, 22.0, 15.0, "#ff6a6a", true, "Away from the shop: purchases wait in the stash");
    }
    let courier = match s.courier {
        0 => "Courier: idle",
        1 => "Courier: delivering",
        2 => "Courier: going home",
        _ => "Courier: dead",
    };
    let burst = if s.courier_cooldown > 0.0 { format!("Burst in {:.0}s", s.courier_cooldown) } else { "Burst ready".to_string() };
    doc.text(
        x + 178.0,
        y + win_h - 20.0,
        win_w - 190.0,
        16.0,
        12.0,
        "#9a9486",
        false,
        &format!("{courier}  |  {burst}  |  C deliver stash   V burst   X home"),
    );

    let shops = visible_shops(data);
    let tab = state.tab.min(shops.len().saturating_sub(1));
    for (i, shop) in shops.iter().enumerate() {
        let ty = y + 50.0 + i as f32 * 31.0;
        let (back, text) = if i == tab { ("#3b3220", "#ffe9a8") } else { ("#15161c", "#a9a38f") };
        doc.click(
            x + 12.0,
            ty,
            150.0,
            27.0,
            &format!("background:{back};border:#3a3326;"),
            "border:#e6c36a;",
            "",
            &format!("tab:{i}"),
        );
        doc.text(x + 20.0, ty + 6.0, 136.0, 18.0, 14.0, text, false, &pretty_words(&shop.name));
    }

    let items: Vec<&String> = shops.get(tab).map(|s| s.items.iter().collect()).unwrap_or_default();
    let (cell_w, cell_h) = (128.0, 98.0);
    for (i, name) in items.iter().enumerate() {
        let col = (i % SHOP_COLUMNS) as f32;
        let row = (i / SHOP_COLUMNS) as f32;
        let cx = x + 178.0 + col * (cell_w + 6.0);
        let cy = y + 50.0 + row * (cell_h + 5.0);
        if cy + cell_h > y + win_h - 96.0 {
            break;
        }
        let cost = data.item(name).map_or(0, |d| d.cost);
        let affordable = s.gold >= cost;
        doc.click(
            cx,
            cy,
            cell_w,
            cell_h,
            "background:#14151a;border:#3a3326;",
            "border:#e6c36a;background:#1d1e26;",
            "",
            &format!("buy:{name}"),
        );
        let icon = format!("dota/icons/item/{name}");
        let dim = if affordable { "" } else { "opacity:0.4;" };
        if has_icon(&icon) {
            doc.at(cx + 25.5, cy + 3.0, 77.0, 56.0, &format!("background-image:url({icon});{dim}"), "");
        }
        doc.text(cx + 2.0, cy + 60.0, cell_w - 4.0, 24.0, 10.0, "#d9d2bc", true, &data.display_name(name));
        let color = if affordable { "#ffd84a" } else { "#ff6a6a" };
        doc.text(cx + 2.0, cy + 85.0, cell_w - 4.0, 13.0, 11.0, color, true, &cost.to_string());
    }

    let strip_y = y + win_h - 84.0;
    doc.text(x + 16.0, strip_y - 20.0, 700.0, 16.0, 12.0, "#9a9486", false, "Your items: click one to sell it for half the price");
    for i in 0..9usize {
        let sx = x + 178.0 + i as f32 * 66.0;
        let item = s.items.get(i).and_then(|i| i.as_ref());
        doc.click(
            sx,
            strip_y,
            62.0,
            48.0,
            "background:#14151a;border:#4a4130;",
            "border:#e6c36a;",
            "",
            &format!("sell:{i}"),
        );
        if let Some(item) = item {
            let icon = format!("dota/icons/item/{}", item.name);
            if has_icon(&icon) {
                doc.at(sx + 1.0, strip_y + 1.0, 60.0, 46.0, &format!("background-image:url({icon});"), "");
            } else {
                doc.text(sx, strip_y + 16.0, 62.0, 16.0, 12.0, "#ece6cf", true, &short_name(&item.name, 8));
            }
        }
    }

    doc.text(x + 178.0 + 9.0 * 66.0, strip_y - 20.0, 200.0, 16.0, 12.0, "#9a9486", false, "Stash");
    for i in 0..s.stash.len().min(6) {
        let sx = x + 178.0 + 9.0 * 66.0 + i as f32 * 32.0;
        doc.at(sx, strip_y, 30.0, 48.0, "background:#14151a;border:#4a4130;", "");
        if let Some(item) = s.stash[i].as_ref() {
            let icon = format!("dota/icons/item/{}", item.name);
            if has_icon(&icon) {
                doc.at(sx + 1.0, strip_y + 12.0, 28.0, 22.0, &format!("background-image:url({icon});"), "");
            } else {
                doc.text(sx, strip_y + 16.0, 30.0, 16.0, 9.0, "#ece6cf", true, &short_name(&item.name, 4));
            }
        }
    }

    if let Some(hover) = state.hover.as_deref().and_then(|h| h.strip_prefix("buy:")) {
        if let Some(def) = data.item(hover) {
            let (tx, ty, tw, th) = (x + win_w + 8.0, y + 50.0, 300.0, 200.0);
            doc.at(tx, ty, tw, th, "background:#0b0c11f8;border:#e6c36a;", "");
            doc.text(tx + 10.0, ty + 8.0, tw - 20.0, 22.0, 18.0, "#ffe9a8", false, &data.display_name(hover));
            doc.text(tx + 10.0, ty + 34.0, tw - 20.0, 18.0, 15.0, "#ffd84a", false, &format!("Cost: {}", def.cost));
            if let Some(recipe) = data.items.values().find(|i| i.recipe && i.result == hover) {
                if let Some(parts) = recipe.requirements.first() {
                    let mut line = parts.iter().map(|p| data.display_name(p)).collect::<Vec<_>>().join(" + ");
                    if recipe.cost > 0 {
                        line.push_str(&format!(" + Recipe ({})", recipe.cost));
                    }
                    doc.text(tx + 10.0, ty + 62.0, tw - 20.0, 60.0, 13.0, "#bfb8a2", false, &line);
                }
            }
        }
    }

    doc.html.push_str("</div>");
    (doc.html, doc.css)
}

#[derive(Clone, Debug, Default)]
pub struct PickState {
    pub selected: Option<String>,
    pub hover: Option<String>,
}

pub fn available_heroes(data: &crate::data::Data) -> Vec<&crate::data::HeroDef> {
    let mut out: Vec<&crate::data::HeroDef> = data
        .heroes
        .iter()
        .filter(|h| h.raw.str("Enabled") == Some("1") && !h.name.ends_with("target_dummy"))
        .collect();
    out.sort_by_key(|h| data.display_name(&h.name));
    out
}

pub fn pick_document(
    data: &crate::data::Data,
    taken: &[String],
    state: &PickState,
    width: f32,
    height: f32,
    has_icon: &dyn Fn(&str) -> bool,
) -> (String, String) {
    use crate::data::Attribute;
    let k = height / REFERENCE_HEIGHT;
    let mut doc = Doc {
        k,
        html: String::new(),
        css: String::new(),
        next: 9000,
    };
    doc.css.push_str("#scene{position:relative;font-family:Radiance;font-weight:400;text-shadow:1px 1px #000000;}");
    let _ = write!(doc.html, "<div id=\"scene\" style=\"width:{width:.1}px;height:{height:.1}px\">");
    let (w, h) = (width / k, height / k);
    doc.at(0.0, 0.0, w, h, "background:linear-gradient(to bottom,#0b0e14,#06070a);", "");
    doc.text_f("Reaver", 700, 0.0, 22.0, w, 52.0, 44.0, "#e6d9a6", true, "CHOOSE YOUR HERO");

    let columns = [
        (Attribute::Strength, "STRENGTH", "#ec3d06"),
        (Attribute::Agility, "AGILITY", "#26e030"),
        (Attribute::Intelligence, "INTELLIGENCE", "#00d9ec"),
        (Attribute::Universal, "UNIVERSAL", "#e6c36a"),
    ];
    let (col_w, gap, cell_w, cell_h, per_row) = (440.0, 24.0, 84.0, 52.0, 5usize);
    let total = columns.len() as f32 * col_w + (columns.len() as f32 - 1.0) * gap;
    let x0 = (w - total) / 2.0;
    let heroes = available_heroes(data);
    for (c, (attr, title, color)) in columns.iter().enumerate() {
        let cx = x0 + c as f32 * (col_w + gap);
        doc.text_f("Reaver", 600, cx, 94.0, col_w, 30.0, 24.0, color, false, title);
        doc.at(cx, 124.0, col_w, 2.0, &format!("background:{color};"), "");
        let list: Vec<&&crate::data::HeroDef> = heroes.iter().filter(|h| h.primary == *attr).collect();
        for (i, hero) in list.iter().enumerate() {
            let x = cx + (i % per_row) as f32 * (cell_w + 4.0);
            let y = 136.0 + (i / per_row) as f32 * (cell_h + 4.0);
            if y + cell_h > h - 330.0 {
                break;
            }
            let is_taken = taken.contains(&hero.name);
            let chosen = state.selected.as_deref() == Some(hero.name.as_str());
            let edge = if chosen { "border:#ffd84a;" } else { "border:#2a2d36;" };
            let portrait = format!("dota/icons/hero/{}", hero.name);
            let dim = if is_taken { "opacity:0.3;" } else { "" };
            let extra = format!("background:#101218;{edge}");
            if is_taken {
                doc.at(x, y, cell_w, cell_h, &extra, "");
            } else {
                doc.click(x, y, cell_w, cell_h, &extra, "border:#e6c36a;", "", &format!("pick:{}", hero.name));
            }
            if has_icon(&portrait) {
                doc.at(x + 1.0, y + 1.0, cell_w - 2.0, cell_h - 2.0, &format!("background-image:url({portrait});{dim}"), "");
            } else {
                doc.text(x, y + 18.0, cell_w, 16.0, 11.0, "#d9d2bc", true, &short_name(&hero.name, 9));
            }
        }
    }

    let shown = state.hover.as_deref().or(state.selected.as_deref());
    let panel_y = h - 292.0;
    doc.at(w * 0.5 - 520.0, panel_y, 1040.0, 262.0, "background:#0c0e13;border:#6b5a35;", "");
    match shown.and_then(|n| data.hero(n)) {
        Some(hero) => {
            let portrait = format!("dota/icons/hero/{}", hero.name);
            if has_icon(&portrait) {
                doc.at(w * 0.5 - 500.0, panel_y + 20.0, 320.0, 180.0, &format!("background-image:url({portrait});"), "");
            }
            doc.text_f("Reaver", 700, w * 0.5 - 500.0, panel_y + 208.0, 320.0, 40.0, 32.0, "#f0e6c8", true, &data.display_name(&hero.name));
            let attr = match hero.primary {
                Attribute::Strength => ("STRENGTH", "#ec3d06"),
                Attribute::Agility => ("AGILITY", "#26e030"),
                Attribute::Intelligence => ("INTELLIGENCE", "#00d9ec"),
                Attribute::Universal => ("UNIVERSAL", "#e6c36a"),
            };
            doc.text_f("Reaver", 600, w * 0.5 - 150.0, panel_y + 20.0, 400.0, 28.0, 22.0, attr.1, false, attr.0);
            let stats = format!(
                "HP {:.0}   Damage {:.0}-{:.0}   Armor {:.1}   Move {:.0}",
                hero.health + hero.base[0] * 22.0,
                hero.damage_min + hero.base[match hero.primary { Attribute::Agility => 1, Attribute::Intelligence => 2, _ => 0 }],
                hero.damage_max + hero.base[match hero.primary { Attribute::Agility => 1, Attribute::Intelligence => 2, _ => 0 }],
                hero.armor + hero.base[1] * 0.16,
                hero.move_speed
            );
            doc.number(w * 0.5 - 150.0, panel_y + 56.0, 700.0, 24.0, 18.0, "#d9d2bc", false, &stats);
            for (i, ability) in hero.abilities.iter().take(5).enumerate() {
                let ax = w * 0.5 - 150.0 + i as f32 * 84.0;
                let icon = format!("dota/icons/spell/{ability}");
                doc.at(ax, panel_y + 100.0, 72.0, 72.0, "background:#14151a;border:#4a4130;", "");
                if has_icon(&icon) {
                    doc.at(ax + 1.0, panel_y + 101.0, 70.0, 70.0, &format!("background-image:url({icon});"), "");
                }
                doc.text(ax - 4.0, panel_y + 176.0, 80.0, 28.0, 11.0, "#bfb8a2", true, &data.display_name(ability));
            }
        }
        None => {
            doc.text_f("Reaver", 600, w * 0.5 - 520.0, panel_y + 100.0, 1040.0, 40.0, 28.0, "#8a8576", true, "Pick a hero");
        }
    }
    if state.selected.is_some() {
        doc.click(
            w * 0.5 + 330.0,
            panel_y + 196.0,
            170.0,
            50.0,
            "background:linear-gradient(to bottom,#c9a64a,#7a5f1c);border:#f0d98a;",
            "border:#ffffff;",
            "",
            "lock",
        );
        doc.text_f("Reaver", 700, w * 0.5 + 330.0, panel_y + 207.0, 170.0, 34.0, 26.0, "#1a1204", true, "LOCK IN");
    }
    doc.html.push_str("</div>");
    (doc.html, doc.css)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{AbilityView, ItemView, TeamHeroView};
    use icraft_ui::{Op, paint};

    fn sample() -> Snapshot {
        Snapshot {
            tick: 9000,
            player: 1,
            hero: "npc_dota_hero_axe".into(),
            radiant: true,
            alive: true,
            level: 3,
            xp: 700,
            xp_this_level: 640,
            xp_next_level: 1160,
            gold: 1133,
            skill_points: 1,
            at_shop: true,
            hp: 500.0,
            max_hp: 670.0,
            hp_regen: 4.5,
            mana: 200.0,
            max_mana: 291.0,
            mana_regen: 0.9,
            strength: 25.0,
            agility: 18.0,
            intelligence: 18.0,
            armor: 2.9,
            magic_resist: 27.0,
            damage_min: 56.0,
            damage_max: 60.0,
            attack_speed: 118.0,
            move_speed: 315.0,
            abilities: vec![
                AbilityView { name: "axe_berserkers_call".into(), level: 1, max_level: 4, cooldown: 7.0, cooldown_total: 18.0, mana: 90.0, passive: false, can_level: true },
                AbilityView { name: "axe_battle_hunger".into(), level: 0, max_level: 4, cooldown: 0.0, cooldown_total: 0.0, mana: 50.0, passive: false, can_level: true },
                AbilityView { name: "axe_counter_helix".into(), level: 0, max_level: 4, cooldown: 0.0, cooldown_total: 0.0, mana: 0.0, passive: true, can_level: false },
                AbilityView { name: "axe_culling_blade".into(), level: 0, max_level: 3, cooldown: 0.0, cooldown_total: 0.0, mana: 60.0, passive: false, can_level: false },
            ],
            items: vec![Some(ItemView { name: "item_tango".into(), charges: 3, cooldown: 0.0 }), None, None, None, None, None, None, None, None],
            team: vec![
                TeamHeroView { player: 1, hero: "npc_dota_hero_axe".into(), radiant: true, level: 3, hp_fraction: 0.75, alive: true, respawn_in: 0.0, kills: 1, deaths: 0, assists: 2 },
                TeamHeroView { player: 2, hero: "npc_dota_hero_pudge".into(), radiant: false, level: 2, hp_fraction: 0.0, alive: false, respawn_in: 14.0, kills: 0, deaths: 1, assists: 0 },
            ],
            ..Snapshot::default()
        }
    }

    fn lay_out(s: &Snapshot, w: f32, h: f32) -> icraft_ui::Paint {
        let (html, css) = document(s, w, h, &|_| true);
        paint(&html, &css, w, h, -1.0, -1.0)
    }

    #[test]
    fn the_panel_stays_inside_the_screen_at_common_sizes() {
        for (w, h) in [(1280.0, 720.0), (1920.0, 1080.0), (427.0, 240.0), (2560.0, 1080.0)] {
            let p = lay_out(&sample(), w, h);
            assert!(!p.ops.is_empty());
            for op in &p.ops {
                let (x, y, ow, oh) = match op {
                    Op::Fill(f) => (f.x, f.y, f.w, f.h),
                    Op::Gradient(g) => (g.x, g.y, g.w, g.h),
                    Op::Image(i) => (i.x, i.y, i.w, i.h),
                    Op::Label(_) | Op::Icon(_) => continue,
                };
                let slack = h * 0.06;
                assert!(x >= -slack && y >= -slack && x + ow <= w + slack && y + oh <= h + slack, "{w}x{h}: {x} {y} {ow} {oh}");
            }
        }
    }

    #[test]
    fn the_bottom_bar_is_145_dota_pixels_tall_and_sits_on_the_bottom_edge() {
        let (w, h) = (1920.0, 1080.0);
        let p = lay_out(&sample(), w, h);
        let bg = p.images.iter().find(|i| i.src == "dota/hud/reborn/ability_bg").unwrap();
        assert!((bg.h - 145.0).abs() < 0.5, "{}", bg.h);
        assert!((bg.y + bg.h - h).abs() < 0.5);
    }

    #[test]
    fn one_panel_per_ability_and_the_level_up_frame_only_where_a_point_can_go() {
        let p = lay_out(&sample(), 1920.0, 1080.0);
        let learn = p.images.iter().filter(|i| i.src == "dota/hud/learnspell").count();
        assert_eq!(learn, 2);
        let passive = p.images.iter().filter(|i| i.src == "dota/hud/passive_ability_border").count();
        let active = p.images.iter().filter(|i| i.src == "dota/hud/reborn/active_ability_border").count();
        assert_eq!((passive, active), (1, 3));
        for key in ["Q", "E", "R", "F"] {
            assert!(p.labels.iter().any(|l| l.text == key), "{key}");
        }
    }

    #[test]
    fn numbers_show_the_snapshot_values() {
        let p = lay_out(&sample(), 1920.0, 1080.0);
        let has = |t: &str| p.labels.iter().any(|l| l.text == t);
        assert!(has("500 / 670"));
        assert!(has("200 / 291"));
        assert!(has("1133"));
        assert!(has("56-60"));
        assert!(has("7.0"));
        assert!(has("Axe"));
    }

    #[test]
    fn the_health_bar_fill_follows_the_fraction() {
        let mut s = sample();
        let width_of = |s: &Snapshot| {
            let p = lay_out(s, 1920.0, 1080.0);
            p.gradients.iter().filter(|g| g.top == 0xFF42_5D25 || g.bottom == 0xFF42_5D25).map(|g| g.w).fold(0.0_f32, f32::max)
        };
        let half = {
            s.hp = 335.0;
            width_of(&s)
        };
        let full = {
            s.hp = 670.0;
            width_of(&s)
        };
        assert!(full > half * 1.9 && full < half * 2.1, "{half} {full}");
    }

    #[test]
    fn a_dead_hero_gets_the_respawn_overlay() {
        let mut s = sample();
        s.alive = false;
        s.respawn_in = 12.4;
        let p = lay_out(&s, 1920.0, 1080.0);
        assert!(p.labels.iter().any(|l| l.text == "Respawn in 13"));
    }

    #[test]
    fn every_texture_the_document_names_is_in_the_install_list() {
        let (html, css) = document(&sample(), 1920.0, 1080.0, &|_| false);
        for text in [html, css] {
            let mut rest = text.as_str();
            while let Some(at) = rest.find("dota/hud/") {
                let tail = &rest[at + "dota/hud/".len()..];
                let end = tail.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '/')).unwrap_or(tail.len());
                assert!(TEXTURES.contains(&&tail[..end]), "{} missing from TEXTURES", &tail[..end]);
                rest = &tail[end..];
            }
        }
    }

    #[test]
    fn the_minimap_places_each_marker_inside_the_frame() {
        use crate::wire::{MARKER_CREEP, MARKER_HERO, Marker};
        let mut s = sample();
        let radiant_home = [-226.0, 203.0, -170.0];
        let dire_home = [117.0, 203.0, 188.0];
        s.markers = vec![
            Marker { x: radiant_home[0], z: radiant_home[2], kind: MARKER_HERO, radiant: true, you: true },
            Marker { x: dire_home[0], z: dire_home[2], kind: MARKER_CREEP, radiant: false, you: false },
        ];
        let p = lay_out(&s, 1920.0, 1080.0);
        let map = p.images.iter().find(|i| i.src == "dota/hud/overview").unwrap();
        assert!((map.w - 228.0).abs() < 0.5 && (map.x - 8.0).abs() < 0.5);
        let green: Vec<_> = p.fills.iter().filter(|f| f.color == 0xFF4C_D964).collect();
        let red: Vec<_> = p.fills.iter().filter(|f| f.color == 0xFFFF_4D4D).collect();
        assert!(green.iter().any(|f| (f.w - 12.0).abs() < 0.1));
        assert!(red.iter().any(|f| (f.w - 5.0).abs() < 0.1));
        let hero = green.iter().find(|f| (f.w - 12.0).abs() < 0.1).unwrap();
        let cx = hero.x + hero.w / 2.0;
        let cy = hero.y + hero.h / 2.0;
        assert!(cx > map.x && cx < map.x + map.w * 0.4, "radiant should be on the left: {cx}");
        assert!(cy > map.y + map.h * 0.6 && cy < map.y + map.h, "radiant should be at the bottom: {cy}");
    }

    #[test]
    fn a_finished_game_shows_the_victory_banner() {
        let mut s = sample();
        let p = lay_out(&s, 1920.0, 1080.0);
        assert!(!p.labels.iter().any(|l| l.text.contains("VICTORY")));
        s.winner = 2;
        let p = lay_out(&s, 1920.0, 1080.0);
        assert!(p.labels.iter().any(|l| l.text == "DIRE VICTORY"));
    }

    #[test]
    fn the_pick_screen_offers_every_free_hero_and_locks_only_after_a_choice() {
        let Some(data) = real_data() else { return };
        let heroes = available_heroes(&data);
        assert!(heroes.len() > 100, "{}", heroes.len());
        let taken = vec![heroes[0].name.clone()];
        let layout = |state: &PickState| {
            let (html, css) = pick_document(&data, &taken, state, 1920.0, 1080.0, &|_| true);
            icraft_ui::paint(&html, &css, 1920.0, 1080.0, -1.0, -1.0)
        };
        let blank = layout(&PickState::default());
        let picks: Vec<&str> = blank.clicks.iter().filter(|c| c.onclick.starts_with("pick:")).map(|c| c.onclick.as_str()).collect();
        assert!(picks.len() > 90, "{}", picks.len());
        assert!(!picks.iter().any(|p| p.ends_with(&heroes[0].name)), "a taken hero cannot be picked");
        assert!(!blank.clicks.iter().any(|c| c.onclick == "lock"));
        let chosen = layout(&PickState { selected: Some("npc_dota_hero_axe".into()), hover: None });
        assert!(chosen.clicks.iter().any(|c| c.onclick == "lock"));
        assert!(chosen.labels.iter().any(|l| l.text == "Axe"));
        assert!(chosen.images.iter().any(|i| i.src == "dota/icons/spell/axe_berserkers_call"));
    }

    #[test]
    fn the_pick_screen_fits_the_screen_and_hover_wins_over_the_choice() {
        let Some(data) = real_data() else { return };
        for (w, h) in [(1920.0, 1080.0), (1280.0, 720.0), (2560.0, 1080.0)] {
            let state = PickState { selected: Some("npc_dota_hero_axe".into()), hover: Some("npc_dota_hero_sven".into()) };
            let (html, css) = pick_document(&data, &[], &state, w, h, &|_| true);
            let p = icraft_ui::paint(&html, &css, w, h, -1.0, -1.0);
            for c in &p.clicks {
                assert!(c.x >= 0.0 && c.y >= 0.0 && c.x + c.w <= w + 1.0 && c.y + c.h <= h + 1.0, "{w}x{h} {c:?}");
            }
            assert!(p.labels.iter().any(|l| l.text == "Sven"));
            assert!(!p.labels.iter().any(|l| l.text == "Axe"));
        }
    }

    #[test]
    fn long_text_never_breaks_the_markup() {
        let mut s = sample();
        s.hero = "npc_dota_hero_<b>&evil".into();
        let p = lay_out(&s, 1280.0, 720.0);
        assert!(p.labels.iter().any(|l| l.text.contains('<')));
    }

    fn real_data() -> Option<crate::data::Data> {
        crate::data::Data::load(&crate::data::dev_data_dir()?).ok()
    }

    fn shop_paint(s: &Snapshot, data: &crate::data::Data, state: &ShopState, mouse: (f32, f32)) -> icraft_ui::Paint {
        let (html, css) = shop_document(s, data, state, 1920.0, 1080.0, &|_| true);
        icraft_ui::paint(&html, &css, 1920.0, 1080.0, mouse.0, mouse.1)
    }

    #[test]
    fn the_shop_lists_every_tab_item_and_inventory_slot_as_a_click_target() {
        let Some(data) = real_data() else { return };
        let s = sample();
        let state = ShopState::default();
        let p = shop_paint(&s, &data, &state, (-1.0, -1.0));
        let tabs = p.clicks.iter().filter(|c| c.onclick.starts_with("tab:")).count();
        let buys: Vec<&str> = p.clicks.iter().filter(|c| c.onclick.starts_with("buy:")).map(|c| c.onclick.as_str()).collect();
        let sells = p.clicks.iter().filter(|c| c.onclick.starts_with("sell:")).count();
        let shops = visible_shops(&data);
        assert_eq!(tabs, shops.len());
        assert!(shops.iter().any(|s| s.name == "consumables"));
        assert!(!shops.iter().any(|s| s.name.contains("pregame")));
        assert_eq!(sells, 9);
        assert_eq!(buys.len(), shops[0].items.len());
        assert!(buys.contains(&"buy:item_tango"));
    }

    #[test]
    fn the_shop_window_sits_above_the_bottom_panel_and_inside_the_screen() {
        let Some(data) = real_data() else { return };
        let p = shop_paint(&sample(), &data, &ShopState::default(), (-1.0, -1.0));
        for c in &p.clicks {
            assert!(c.x >= 0.0 && c.y >= 0.0 && c.x + c.w <= 1920.0 && c.y + c.h <= 1080.0 - 145.0 + 1.0, "{c:?}");
        }
    }

    #[test]
    fn what_you_cannot_afford_is_dimmed_and_red() {
        let Some(data) = real_data() else { return };
        let mut s = sample();
        s.gold = 10;
        let poor = shop_paint(&s, &data, &ShopState { tab: 0, hover: None }, (-1.0, -1.0));
        assert!(poor.labels.iter().any(|l| l.color == 0xFFFF_6A6A));
        assert!(poor.images.iter().any(|i| i.opacity < 0.5));
        s.gold = 100000;
        let rich = shop_paint(&s, &data, &ShopState { tab: 0, hover: None }, (-1.0, -1.0));
        assert!(!rich.labels.iter().any(|l| l.color == 0xFFFF_6A6A));
    }

    #[test]
    fn a_far_player_is_told_to_go_back() {
        let Some(data) = real_data() else { return };
        let mut s = sample();
        s.at_shop = false;
        let p = shop_paint(&s, &data, &ShopState::default(), (-1.0, -1.0));
        assert!(p.labels.iter().any(|l| l.text.contains("Away from the shop")));
        s.at_shop = true;
        let p = shop_paint(&s, &data, &ShopState::default(), (-1.0, -1.0));
        assert!(!p.labels.iter().any(|l| l.text.contains("Away from the shop")));
    }

    #[test]
    fn hovering_an_item_shows_its_recipe() {
        let Some(data) = real_data() else { return };
        let consumables = data.shops.iter().position(|s| s.items.iter().any(|i| i == "item_blade_mail"));
        let state = ShopState { tab: consumables.unwrap_or(0), hover: Some("buy:item_blade_mail".into()) };
        let p = shop_paint(&sample(), &data, &state, (-1.0, -1.0));
        assert!(p.labels.iter().any(|l| l.text == "Blade Mail"));
        assert!(p.labels.iter().any(|l| l.text.contains("Recipe")));
    }

    #[test]
    fn tabs_switch_the_item_list() {
        let Some(data) = real_data() else { return };
        let first = shop_paint(&sample(), &data, &ShopState { tab: 0, hover: None }, (-1.0, -1.0));
        let second = shop_paint(&sample(), &data, &ShopState { tab: 1, hover: None }, (-1.0, -1.0));
        let items = |p: &icraft_ui::Paint| p.clicks.iter().filter(|c| c.onclick.starts_with("buy:")).map(|c| c.onclick.clone()).collect::<Vec<_>>();
        assert_ne!(items(&first), items(&second));
    }
}
