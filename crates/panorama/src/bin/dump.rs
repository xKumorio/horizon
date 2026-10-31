use std::path::PathBuf;

fn png_size(path: &PathBuf) -> Option<(f32, f32)> {
    let b = std::fs::read(path).ok()?;
    if b.len() < 24 {
        return None;
    }
    let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
    let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
    Some((w as f32, h as f32))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let layout = args.get(1).map_or("layout/hud/hud_reborn.xml", String::as_str);
    let out = args.get(2).map_or("dump.txt", String::as_str);
    let (w, h): (f32, f32) = (1920.0, 1080.0);
    let data = PathBuf::from(r"C:\Users\root\Desktop\rust\dota-game-data");
    let mut doc = panorama::Doc::open(&data.join("panorama").join("panorama"), layout).expect("layout");
    if let Ok(keep) = std::env::var("KEEP") {
        let ids: Vec<&str> = keep.split(',').collect();
        doc.keep_only(&ids);
    }
    let quiet = ["InvokerSpellCard", "RingmasterSouvenirCard", "KezWeaponCard", "LargoSongCard", "AbilityDetailsUICard", "LargoUltimateRhythm", "InvokerAghsStatusContainer", "StatBranchDrawer", "KillGraphButton", "unitbadge", "RecommendedUpgradeOverlay", "MorphProgress", "death_panel_buyback", "buffs", "debuffs", "UnitFrames", "StatBranchHotkey", "SecondaryAbilityContainer", "StashContainer"];
    doc.hide(&quiet);
    doc.fill_slots("abilities", 4, "DOTAAbilityPanel");
    doc.fill_slots("inventory_list", 3, "DOTAAbilityPanel");
    doc.fill_slots("inventory_list2", 3, "DOTAAbilityPanel");
    doc.fill_slots("inventory_backpack_list", 3, "DOTAAbilityPanel");
    eprintln!("panels {}", doc.root.count());
    let images = data.join("hud_images").join("panorama").join("images");
    let measure = |text: &str, font: &icraft_ui::Font| text.chars().count() as f32 * font.size * 0.52;
    let resolve = |url: &str| -> Option<(String, f32, f32)> {
        let key = panorama::image_key(url)?;
        let path = images.join(format!("{key}.png"));
        let (iw, ih) = png_size(&path)?;
        Some((path.to_string_lossy().into_owned(), iw, ih))
    };
    let paint = doc.render(w, h, &measure, &resolve);
    if let Ok(id) = std::env::var("INSPECT") {
        if let Some(p) = doc.root.find(&id) {
            eprintln!("{} rect {:?} hidden {} classes {:?}", id, p.rect, p.hidden, p.classes);
            let mut keys: Vec<_> = p.style.iter().collect();
            keys.sort();
            for (k, v) in keys {
                eprintln!("  {k}: {v}");
            }
        }
        eprintln!("root classes {:?} style vis {:?}", doc.root.classes, doc.root.style.get("visibility"));
    }
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
            show(p, 0, 6);
        }
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
    eprintln!("ops {}", paint.ops.len());
}
