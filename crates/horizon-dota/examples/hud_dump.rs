use horizon_dota::panorama_hud;
use horizon_dota::wire::{AbilityView, ItemView, Marker, Snapshot};
use std::path::PathBuf;

fn png_size(path: &PathBuf) -> Option<(f32, f32)> {
    let b = std::fs::read(path).ok()?;
    if b.len() < 24 {
        return None;
    }
    Some((u32::from_be_bytes([b[16], b[17], b[18], b[19]]) as f32, u32::from_be_bytes([b[20], b[21], b[22], b[23]]) as f32))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).map_or("dump.txt", String::as_str);
    let data = PathBuf::from(r"C:\Users\root\Desktop\rust\dota-game-data");
    let mut doc = panorama::Doc::open(&data.join("panorama").join("panorama"), "layout/hud/hud_reborn.xml").expect("layout");
    panorama_hud::prepare(&mut doc);
    let ability = |n: &str, level, cd| AbilityView { name: n.into(), level, max_level: 4, cooldown: cd, cooldown_total: 20.0, mana: 80.0, passive: false, can_level: level == 0 };
    let s = Snapshot {
        hero: "npc_dota_hero_axe".into(),
        radiant: true,
        alive: true,
        level: 6,
        xp: 900,
        xp_this_level: 800,
        xp_next_level: 1000,
        gold: 1089,
        hp: 540.0,
        max_hp: 670.0,
        hp_regen: 4.5,
        mana: 200.0,
        max_mana: 291.0,
        mana_regen: 0.9,
        armor: 2.9,
        damage_min: 56.0,
        damage_max: 60.0,
        attack_speed: 118.0,
        move_speed: 315.0,
        strength: 25.0,
        agility: 18.0,
        intelligence: 18.0,
        abilities: vec![ability("axe_berserkers_call", 2, 0.0), ability("axe_battle_hunger", 1, 7.0), ability("axe_counter_helix", 0, 0.0), ability("axe_culling_blade", 1, 0.0)],
        items: vec![Some(ItemView { name: "item_tango".into(), charges: 3, cooldown: 0.0 }), Some(ItemView { name: "item_blink".into(), charges: 0, cooldown: 5.0 }), None, None, None, None],
        team: (0..10).map(|i| horizon_dota::wire::TeamHeroView { player: i, hero: if i % 2 == 0 { "npc_dota_hero_axe".into() } else { "npc_dota_hero_lina".into() }, radiant: i < 5, level: 3, hp_fraction: 0.7, alive: i != 3, respawn_in: 12.0, kills: 2, deaths: 1, assists: 0 }).collect(),
        tick: 30 * 754,
        markers: vec![Marker { x: -100.0, z: 0.0, kind: 0, radiant: true, you: true }],
        ..Snapshot::default()
    };
    let dots = panorama_hud::minimap_dots(&s);
    panorama_hud::bind(&mut doc, &s, "Axe", &dots);
    let images = data.join("hud_images").join("panorama").join("images");
    let icons = PathBuf::from(r"C:\Users\root\Desktop\rust\client\.local\horizon-client\26.3\assets\minecraft\textures\gui\dota");
    let measure = |text: &str, font: &icraft_ui::Font| text.chars().count() as f32 * font.size * 0.52;
    let resolve = |url: &str| -> Option<(String, f32, f32)> {
        let path = if let Some(rest) = url.strip_prefix("dota:") { icons.join(format!("{rest}.png")) } else { images.join(format!("{}.png", panorama::image_key(url)?)) };
        let (w, h) = png_size(&path)?;
        Some((path.to_string_lossy().into_owned(), w, h))
    };
    let paint = doc.render(1920.0, 1080.0, &measure, &resolve);
    if let Ok(id) = std::env::var("TREE") {
        fn show(p: &panorama::Panel, depth: usize, max: usize) {
            let vis = p.style.get("visibility").map_or("", String::as_str);
            if p.hidden || vis == "collapse" {
                return;
            }
            let r = &p.rect;
            println!("{}{} #{} .{} [{:.0},{:.0} {:.0}x{:.0}]", "  ".repeat(depth), p.tag, p.id, p.classes.join("."), r.x, r.y, r.w, r.h);
            if depth < max {
                for c in &p.children {
                    show(c, depth + 1, max);
                }
            }
        }
        if let Some(p) = doc.root.find(&id) {
            show(p, 0, 5);
        }
    }
    if let Ok(id) = std::env::var("STYLE") {
        if let Some(p) = doc.root.find(&id) {
            let mut keys: Vec<_> = p.style.iter().collect();
            keys.sort();
            for (k, v) in keys {
                println!("{k}: {v}");
            }
        }
    }
    if let Ok(id) = std::env::var("ALL") {
        fn walk(p: &panorama::Panel, id: &str, path: &str) {
            let here = format!("{path}/{}#{}", p.tag, p.id);
            if p.id == id {
                println!("{here} hidden {} vis {:?} rect {:?}", p.hidden, p.style.get("visibility"), p.rect);
            }
            for c in &p.children {
                walk(c, id, &here);
            }
        }
        walk(&doc.root, &id, "");
    }
    let mut text = String::new();
    for op in &paint.ops {
        match op {
            icraft_ui::Op::Fill(f) => text.push_str(&format!("F {} {} {} {} {}\n", f.x, f.y, f.w, f.h, f.color)),
            icraft_ui::Op::Image(i) => text.push_str(&format!("I {} {} {} {} {} {}\n", i.x, i.y, i.w, i.h, i.opacity, i.src)),
            icraft_ui::Op::Label(l) => text.push_str(&format!("L {} {} {} {} {}|{}\n", l.x, l.y, l.size, l.color, l.family, l.text)),
            _ => {}
        }
    }
    std::fs::write(out, text).unwrap();
    std::fs::write(format!("{out}.trace"), doc.trace.join("
")).unwrap();
    eprintln!("ops {}", paint.ops.len());
}
