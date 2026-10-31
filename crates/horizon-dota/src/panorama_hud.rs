use crate::wire::Snapshot;
use panorama::{Doc, Panel};

pub const QUIET: [&str; 21] = [
    "multiunit",
    "InvokerSpellCard",
    "RingmasterSouvenirCard",
    "KezWeaponCard",
    "LargoSongCard",
    "AbilityDetailsUICard",
    "LargoUltimateRhythm",
    "InvokerAghsStatusContainer",
    "StatBranchDrawer",
    "KillGraphButton",
    "unitbadge",
    "RecommendedUpgradeOverlay",
    "MorphProgress",
    "death_panel_buyback",
    "buffs",
    "debuffs",
    "UnitFrames",
    "StatBranchHotkey",
    "SecondaryAbilityContainer",
    "StashContainer",
    "AghsStatusContainer",
];

pub const KEPT: [&str; 3] = ["lower_hud", "minimap_container", "topbar"];
pub const ABILITY_SLOTS: usize = 4;
const HOTKEYS: [&str; 6] = ["Q", "E", "R", "F", "G", "H"];

fn text(doc: &mut Doc, id: &str, value: &str) {
    doc.set_text(id, value);
}

fn slot_icon(slot: &mut Panel, image_id: &str, src: &str) {
    if let Some(p) = slot.find_mut(image_id) {
        p.image = src.to_string();
    }
}

fn level_pips(slot: &mut Panel, max: u32, learned: u32) {
    if let Some(c) = slot.find_mut("AbilityLevelContainer") {
        c.children = (0..max)
            .map(|i| {
                let mut p = Panel::new("Panel", &["LevelPanel"]);
                if i < learned {
                    p.set_class("active_level", true);
                }
                p
            })
            .collect();
    }
}

fn bind_ability(slot: &mut Panel, a: Option<&crate::wire::AbilityView>, key: &str, mana_ok: f32) {
    let Some(a) = a else {
        slot.hidden = true;
        return;
    };
    slot.hidden = false;
    slot_icon(slot, "AbilityImage", &format!("dota:icons/spell/{}", a.name));
    let learned = a.level > 0;
    slot.set_class("no_level", !learned);
    slot.set_class("is_passive", a.passive);
    slot.set_class("could_level_up", a.can_level);
    slot.set_class("show_level_up_tab", a.can_level);
    slot.set_class("in_cooldown", a.cooldown > 0.0);
    slot.set_class("no_mana_cost", a.mana <= 0.0);
    slot.set_class("no_health_cost", true);
    slot.set_class("no_gold_cost", true);
    slot.set_class("insufficient_mana", learned && a.mana > mana_ok);
    if let Some(p) = slot.find_mut("CooldownTimer") {
        p.text = if a.cooldown > 0.0 { format!("{}", a.cooldown.ceil() as i32) } else { String::new() };
    }
    if let Some(p) = slot.find_mut("ManaCost") {
        p.text = format!("{}", a.mana.round() as i32);
    }
    if let Some(p) = slot.find_mut("HotkeyText") {
        p.text = key.to_string();
    }
    level_pips(slot, a.max_level.max(1), a.level);
}

fn bind_item(slot: &mut Panel, item: Option<&crate::wire::ItemView>, key: &str) {
    slot.set_class("InventoryItem", true);
    match item {
        Some(i) => {
            slot_icon(slot, "ItemImage", &format!("dota:icons/item/{}", i.name));
            slot.set_class("no_level", false);
            slot.set_class("in_cooldown", i.cooldown > 0.0);
            slot.set_class("no_mana_cost", true);
            slot.set_class("no_health_cost", true);
            slot.set_class("no_gold_cost", true);
            if let Some(p) = slot.find_mut("CooldownTimer") {
                p.text = if i.cooldown > 0.0 { format!("{}", i.cooldown.ceil() as i32) } else { String::new() };
            }
            if let Some(p) = slot.find_mut("ItemCharges") {
                p.text = if i.charges > 1 { i.charges.to_string() } else { String::new() };
            }
        }
        None => {
            slot_icon(slot, "ItemImage", "");
            slot.set_class("no_level", true);
        }
    }
    if let Some(p) = slot.find_mut("HotkeyText") {
        p.text = key.to_string();
    }
}

pub fn bind(doc: &mut Doc, s: &Snapshot, hero_name: &str, minimap: &[(f32, f32, u32, f32)]) {
    let hp = s.hp.max(0.0);
    text(doc, "HealthLabel", &format!("{} / {}", hp.round() as i32, s.max_hp.round() as i32));
    text(doc, "ManaLabel", &format!("{} / {}", s.mana.round() as i32, s.max_mana.round() as i32));
    text(doc, "HealthRegenLabel", &format!("+{:.1}", s.hp_regen));
    text(doc, "ManaRegenLabel", &format!("+{:.1}", s.mana_regen));
    doc.set_value("HealthProgress", hp / s.max_hp.max(1.0));
    doc.set_value("ManaProgress", s.mana / s.max_mana.max(1.0));
    bind_top_bar(doc, s);
    text(doc, "GoldLabel", &s.gold.to_string());
    if let Some(p) = doc.panel("stats") {
        p.set_class("ShowDamageArmorMovement", true);
        p.set_class("ShowSplitLabels", true);
    }
    if let Some(p) = doc.panel("stragiint") {
        p.set_class("ShowStrAgiInt", true);
    }
    for id in ["portraitHUD"] {
        if let Some(p) = doc.panel(id) {
            p.style_force("width", "159px");
            p.style_force("height", "145px");
            p.style_force("transform", "none");
        }
    }
    doc.hide(&["portraitHUDOverlay", "RightSideHeroBlur"]);
    let avg = ((s.damage_min + s.damage_max) / 2.0).round() as i32;
    let resist = s.armor * 0.06 / (1.0 + s.armor.abs() * 0.06) * 100.0;
    for (id, value) in [
        ("DamageLabel", avg.to_string()),
        ("DamageLabelBase", avg.to_string()),
        ("AttackSpeedLabelBase", format!("{:.0}", s.attack_speed)),
        ("ArmorLabel", format!("{:.1}", s.armor)),
        ("ArmorLabelBase", format!("{:.1}", s.armor)),
        ("PhysicalDamageResist", format!("{:.0}%", resist)),
        ("MagicResistLabelBase", format!("{:.0}%", s.magic_resist * 100.0)),
        ("MoveSpeedLabelBase", format!("{:.0}", s.move_speed)),
        ("StrengthSumLabel", format!("{:.0}", s.strength)),
        ("StrengthLabel", format!("{:.0}", s.strength)),
        ("AgilitySumLabel", format!("{:.0}", s.agility)),
        ("AgilityLabel", format!("{:.0}", s.agility)),
        ("IntelligenceSumLabel", format!("{:.0}", s.intelligence)),
        ("IntelligenceLabel", format!("{:.0}", s.intelligence)),
    ] {
        text(doc, id, &value);
    }
    text(doc, "LevelLabel", &s.level.to_string());
    text(doc, "UnitNameLabel", hero_name);
    let span = (s.xp_next_level - s.xp_this_level).max(1) as f32;
    doc.set_value("CircularXPProgress", ((s.xp - s.xp_this_level) as f32 / span).clamp(0.0, 1.0));
    if let Some(p) = doc.panel("portraitHUD") {
        p.image = format!("dota:icons/hero/{}", s.hero);
    }
    if let Some(p) = doc.panel("abilities") {
        let n = p.children.len();
        for i in 0..n {
            let a = s.abilities.get(i);
            bind_ability(&mut p.children[i], a, HOTKEYS[i.min(5)], s.mana);
        }
    }
    for (list, base) in [("inventory_list", 0usize), ("inventory_list2", 3usize)] {
        if let Some(p) = doc.panel(list) {
            for (i, slot) in p.children.iter_mut().enumerate() {
                let idx = base + i;
                bind_item(slot, s.items.get(idx).and_then(|x| x.as_ref()), &(idx + 1).to_string());
            }
        }
    }
    if let Some(p) = doc.panel("minimap") {
        p.image = "dota:hud/overview".into();
        p.children.clear();
        for (x, y, color, size) in minimap {
            let mut dot = Panel::new("Panel", &[]);
            dot.style_force("width", &format!("{size}px"));
            dot.style_force("height", &format!("{size}px"));
            dot.style_force("horizontal-align", "left");
            dot.style_force("vertical-align", "top");
            dot.style_force("margin-left", &format!("{}px", x - size / 2.0));
            dot.style_force("margin-top", &format!("{}px", y - size / 2.0));
            dot.style_force("background-color", &format!("#{:06x}", color & 0xFFFFFF));
            p.children.push(dot);
        }
    }
}


pub fn prepare(doc: &mut Doc) {
    doc.keep_only(&KEPT);
    doc.hide(&QUIET);
    doc.fill_slots("abilities", ABILITY_SLOTS, "DOTAAbilityPanel");
    doc.fill_slots("inventory_list", 3, "DOTAAbilityPanel");
    doc.fill_slots("inventory_list2", 3, "DOTAAbilityPanel");
    doc.fill_slots("inventory_backpack_list", 0, "DOTAAbilityPanel");
    doc.fill_slots("TopBarRadiantPlayersContainer", 5, "DOTATopBarPlayer");
    doc.fill_slots("TopBarDirePlayersContainer", 5, "DOTATopBarPlayer");
}

pub fn minimap_dots(s: &Snapshot) -> Vec<(f32, f32, u32, f32)> {
    let mut dots = Vec::new();
    for m in &s.markers {
        let [u, v] = crate::map::minimap_uv([m.x, 0.0, m.z]);
        let color = match m.kind {
            crate::wire::MARKER_NEUTRAL => 0xffd84a,
            crate::wire::MARKER_COURIER => 0x40e0d0,
            crate::wire::MARKER_RUNE => 0xff9be8,
            _ => {
                if m.radiant {
                    0x4cd964
                } else {
                    0xff4d4d
                }
            }
        };
        dots.push((u * 260.0, v * 260.0, color, if m.you { 9.0 } else { 6.0 }));
    }
    dots
}

fn bind_top_bar(doc: &mut Doc, s: &Snapshot) {
    let seconds = (s.tick / 30) as i32;
    text(doc, "GameTime", &format!("{}:{:02}", seconds / 60, seconds % 60));
    for (container, radiant, score_id) in [
        ("TopBarRadiantPlayersContainer", true, "TopBarRadiantScore"),
        ("TopBarDirePlayersContainer", false, "TopBarDireScore"),
    ] {
        let heroes: Vec<&crate::wire::TeamHeroView> = s.team.iter().filter(|t| t.radiant == radiant).collect();
        let kills: u32 = heroes.iter().map(|t| t.kills).sum();
        text(doc, score_id, &kills.to_string());
        if let Some(p) = doc.panel(container) {
            for (i, slot) in p.children.iter_mut().enumerate() {
                let Some(t) = heroes.get(i) else {
                    slot.hidden = true;
                    continue;
                };
                slot.hidden = false;
                slot.set_class("Dead", !t.alive);
                slot.set_class("LocalPlayer", t.player == s.player);
                if let Some(h) = slot.find_mut("HeroImage") {
                    h.image = format!("dota:icons/hero/{}", t.hero);
                }
                if let Some(h) = slot.find_mut("HealthBar") {
                    h.value = t.hp_fraction.clamp(0.0, 1.0);
                }
                if let Some(h) = slot.find_mut("ManaBar") {
                    h.value = 1.0;
                }
                if let Some(h) = slot.find_mut("RespawnTimerLabel") {
                    h.text = if t.alive { String::new() } else { format!("{}", t.respawn_in.ceil() as i32) };
                }
            }
        }
    }
}
