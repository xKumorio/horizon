use crate::kv::{Kv, Value, first_number, numbers};
use std::{collections::HashMap, path::Path};

pub fn at(levels: &[f32], level: usize) -> f32 {
    match levels.len() {
        0 => 0.0,
        n => levels[level.saturating_sub(1).min(n - 1)],
    }
}

#[derive(Clone, Debug, Default)]
pub struct AbilityDef {
    pub name: String,
    pub behavior: Vec<String>,
    pub target_team: Vec<String>,
    pub target_type: Vec<String>,
    pub damage_type: String,
    pub cast_range: Vec<f32>,
    pub cast_point: f32,
    pub cooldown: Vec<f32>,
    pub mana_cost: Vec<f32>,
    pub damage: Vec<f32>,
    pub max_level: u32,
    pub ultimate: bool,
    pub shared_cooldown: String,
    pub values: Vec<(String, Vec<f32>)>,
    pub raw: Kv,
}

fn flags(text: Option<&str>) -> Vec<String> {
    text.map(|t| {
        t.split('|')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

fn list(kv: &Kv, key: &str) -> Vec<f32> {
    kv.str(key).map(numbers).unwrap_or_default()
}

impl AbilityDef {
    pub fn from_kv(name: &str, kv: &Kv) -> AbilityDef {
        let mut values = Vec::new();
        if let Some(block) = kv.block("AbilityValues") {
            for (key, value) in &block.entries {
                let levels = match value {
                    Value::Str(s) => numbers(s),
                    Value::Block(b) => list(b, "value"),
                };
                values.push((key.clone(), levels));
            }
        }
        let ultimate = kv
            .str("AbilityType")
            .is_some_and(|t| t.contains("ULTIMATE"));
        let top_or_value = |key: &str| -> Vec<f32> {
            let top = list(kv, key);
            if !top.is_empty() {
                return top;
            }
            values
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        let cast_range = top_or_value("AbilityCastRange");
        let cooldown = top_or_value("AbilityCooldown");
        let mana_cost = top_or_value("AbilityManaCost");
        AbilityDef {
            name: name.to_string(),
            behavior: flags(kv.str("AbilityBehavior")),
            target_team: flags(kv.str("AbilityUnitTargetTeam")),
            target_type: flags(kv.str("AbilityUnitTargetType")),
            damage_type: kv.str("AbilityUnitDamageType").unwrap_or("").to_string(),
            cast_range,
            cast_point: kv.f32_or("AbilityCastPoint", 0.0),
            cooldown,
            mana_cost,
            damage: list(kv, "AbilityDamage"),
            max_level: kv
                .int("MaxLevel")
                .map(|v| v.max(0) as u32)
                .unwrap_or(if ultimate { 3 } else { 4 }),
            ultimate,
            shared_cooldown: kv.str("AbilitySharedCooldown").unwrap_or("").to_string(),
            values,
            raw: kv.clone(),
        }
    }

    pub fn has_behavior(&self, flag: &str) -> bool {
        self.behavior
            .iter()
            .any(|b| b.strip_prefix("DOTA_ABILITY_BEHAVIOR_") == Some(flag))
    }

    pub fn passive(&self) -> bool {
        self.has_behavior("PASSIVE")
    }

    pub fn hidden(&self) -> bool {
        self.has_behavior("HIDDEN")
    }

    pub fn value(&self, name: &str, level: usize) -> f32 {
        self.values
            .iter()
            .find(|(k, _)| k == name)
            .map_or(0.0, |(_, v)| at(v, level))
    }

    pub fn cooldown_at(&self, level: usize) -> f32 {
        at(&self.cooldown, level)
    }

    pub fn mana_at(&self, level: usize) -> f32 {
        at(&self.mana_cost, level)
    }

    pub fn range_at(&self, level: usize) -> f32 {
        at(&self.cast_range, level)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ItemDef {
    pub name: String,
    pub cost: i32,
    pub recipe: bool,
    pub result: String,
    pub requirements: Vec<Vec<String>>,
    pub stackable: bool,
    pub initial_charges: i32,
    pub neutral_tier: i32,
    pub ability: AbilityDef,
}

impl ItemDef {
    pub fn from_kv(name: &str, kv: &Kv) -> ItemDef {
        let mut requirements = Vec::new();
        if let Some(block) = kv.block("ItemRequirements") {
            for (_, value) in &block.entries {
                if let Value::Str(s) = value {
                    requirements.push(
                        s.split(';')
                            .map(|n| n.trim().trim_end_matches('*').to_string())
                            .filter(|n| !n.is_empty())
                            .collect(),
                    );
                }
            }
        }
        ItemDef {
            name: name.to_string(),
            cost: kv.int("ItemCost").unwrap_or(0),
            recipe: kv.int("ItemRecipe").unwrap_or(0) != 0,
            result: kv.str("ItemResult").unwrap_or("").to_string(),
            requirements,
            stackable: kv.int("ItemStackable").unwrap_or(0) != 0,
            initial_charges: kv.int("ItemInitialCharges").unwrap_or(0),
            neutral_tier: 0,
            ability: AbilityDef::from_kv(name, kv),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct HeroDef {
    pub name: String,
    pub id: i32,
    pub primary: Attribute,
    pub base: [f32; 3],
    pub gain: [f32; 3],
    pub armor: f32,
    pub magic_resist: f32,
    pub damage_min: f32,
    pub damage_max: f32,
    pub attack_rate: f32,
    pub attack_point: f32,
    pub attack_range: f32,
    pub projectile_speed: f32,
    pub melee: bool,
    pub move_speed: f32,
    pub health: f32,
    pub health_regen: f32,
    pub mana: f32,
    pub mana_regen: f32,
    pub vision_day: f32,
    pub vision_night: f32,
    pub abilities: Vec<String>,
    pub talents: Vec<String>,
    pub raw: Kv,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Attribute {
    #[default]
    Strength,
    Agility,
    Intelligence,
    Universal,
}

impl HeroDef {
    pub fn from_kv(name: &str, kv: &Kv) -> HeroDef {
        let primary = match kv.str("AttributePrimary").unwrap_or("") {
            s if s.ends_with("AGILITY") => Attribute::Agility,
            s if s.ends_with("INTELLECT") || s.ends_with("INTELLIGENCE") => Attribute::Intelligence,
            s if s.ends_with("ALL") => Attribute::Universal,
            _ => Attribute::Strength,
        };
        let mut abilities = Vec::new();
        let mut talents = Vec::new();
        for slot in 1..=25 {
            let Some(ability) = kv.str(&format!("Ability{slot}")) else {
                continue;
            };
            if ability.is_empty() || ability == "generic_hidden" {
                continue;
            }
            if ability.starts_with("special_bonus") {
                talents.push(ability.to_string());
            } else if slot <= 9 {
                abilities.push(ability.to_string());
            }
        }
        let drafted: Vec<String> = kv
            .block("AbilityDraftAbilities")
            .map(|block| {
                block
                    .entries
                    .iter()
                    .filter_map(|(_, v)| match v {
                        Value::Str(s) if !s.is_empty() => Some(s.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !drafted.is_empty() {
            abilities = drafted;
        }
        HeroDef {
            name: name.to_string(),
            id: kv.int("HeroID").unwrap_or(0),
            primary,
            base: [
                kv.f32_or("AttributeBaseStrength", 0.0),
                kv.f32_or("AttributeBaseAgility", 0.0),
                kv.f32_or("AttributeBaseIntelligence", 0.0),
            ],
            gain: [
                kv.f32_or("AttributeStrengthGain", 0.0),
                kv.f32_or("AttributeAgilityGain", 0.0),
                kv.f32_or("AttributeIntelligenceGain", 0.0),
            ],
            armor: kv.f32_or("ArmorPhysical", 0.0),
            magic_resist: kv.f32_or("MagicalResistance", 25.0),
            damage_min: kv.f32_or("AttackDamageMin", 0.0),
            damage_max: kv.f32_or("AttackDamageMax", 0.0),
            attack_rate: kv.f32_or("AttackRate", 1.7),
            attack_point: kv.f32_or("AttackAnimationPoint", 0.5),
            attack_range: kv.f32_or("AttackRange", 150.0),
            projectile_speed: kv.f32_or("ProjectileSpeed", 0.0),
            melee: kv
                .str("AttackCapabilities")
                .is_some_and(|c| c.contains("MELEE")),
            move_speed: kv.f32_or("MovementSpeed", 300.0),
            health: kv.f32_or("StatusHealth", 120.0),
            health_regen: kv.f32_or("StatusHealthRegen", 0.25),
            mana: kv.f32_or("StatusMana", 75.0),
            mana_regen: kv.f32_or("StatusManaRegen", 0.0),
            vision_day: kv.f32_or("VisionDaytimeRange", 1800.0),
            vision_night: kv.f32_or("VisionNighttimeRange", 800.0),
            abilities,
            talents,
            raw: kv.clone(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct UnitDef {
    pub name: String,
    pub base_class: String,
    pub health: f32,
    pub health_regen: f32,
    pub mana: f32,
    pub armor: f32,
    pub magic_resist: f32,
    pub damage_min: f32,
    pub damage_max: f32,
    pub attack_rate: f32,
    pub attack_point: f32,
    pub attack_range: f32,
    pub acquisition_range: f32,
    pub projectile_speed: f32,
    pub melee: bool,
    pub move_speed: f32,
    pub bounty_xp: f32,
    pub bounty_gold: (f32, f32),
    pub vision_day: f32,
    pub vision_night: f32,
    pub abilities: Vec<String>,
    pub raw: Kv,
}

impl UnitDef {
    pub fn from_kv(name: &str, kv: &Kv) -> UnitDef {
        UnitDef {
            name: name.to_string(),
            base_class: kv.str("BaseClass").unwrap_or("").to_string(),
            health: kv.f32_or("StatusHealth", 0.0),
            health_regen: kv.f32_or("StatusHealthRegen", 0.0),
            mana: kv.f32_or("StatusMana", 0.0),
            armor: kv.f32_or("ArmorPhysical", 0.0),
            magic_resist: kv.f32_or("MagicalResistance", 0.0),
            damage_min: kv.f32_or("AttackDamageMin", 0.0),
            damage_max: kv.f32_or("AttackDamageMax", 0.0),
            attack_rate: kv.f32_or("AttackRate", 1.0),
            attack_point: kv.f32_or("AttackAnimationPoint", 0.5),
            attack_range: kv.f32_or("AttackRange", 100.0),
            acquisition_range: kv.f32_or("AttackAcquisitionRange", 500.0),
            projectile_speed: kv.str("ProjectileSpeed").and_then(first_number).unwrap_or(0.0),
            melee: kv
                .str("AttackCapabilities")
                .is_some_and(|c| c.contains("MELEE")),
            move_speed: kv.f32_or("MovementSpeed", 300.0),
            bounty_xp: kv.f32_or("BountyXP", 0.0),
            bounty_gold: (
                kv.f32_or("BountyGoldMin", 0.0),
                kv.f32_or("BountyGoldMax", 0.0),
            ),
            vision_day: kv.f32_or("VisionDaytimeRange", 0.0),
            vision_night: kv.f32_or("VisionNighttimeRange", 0.0),
            abilities: (1..=8)
                .filter_map(|i| kv.str(&format!("Ability{i}")))
                .filter(|a| !a.is_empty())
                .map(String::from)
                .collect(),
            raw: kv.clone(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Shop {
    pub name: String,
    pub items: Vec<String>,
}

#[derive(Default)]
pub struct Data {
    pub heroes: Vec<HeroDef>,
    pub abilities: HashMap<String, AbilityDef>,
    pub items: HashMap<String, ItemDef>,
    pub units: HashMap<String, UnitDef>,
    pub shops: Vec<Shop>,
    pub names: HashMap<String, String>,
}

fn load_names(root: &Path, language: &str) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for file in [format!("abilities_{language}.txt"), format!("dota_{language}.txt")] {
        let path = root.join("resource").join("localization").join(file);
        let Ok(kv) = Kv::parse_file(&path) else { continue };
        let Some(tokens) = kv.block("lang").and_then(|l| l.block("Tokens")) else {
            continue;
        };
        for (key, value) in &tokens.entries {
            if let Value::Str(text) = value {
                names.insert(key.to_ascii_lowercase(), text.clone());
            }
        }
    }
    names
}

fn load_shops(root: &Path) -> Vec<Shop> {
    let Ok(kv) = Kv::parse_file(&root.join("scripts").join("shops.txt")) else {
        return Vec::new();
    };
    let Some(block) = kv.block("dota_shops") else {
        return Vec::new();
    };
    block
        .blocks()
        .map(|(name, shop)| Shop {
            name: name.to_string(),
            items: shop
                .entries
                .iter()
                .filter_map(|(k, v)| match v {
                    Value::Str(item) if k == "item" => Some(item.clone()),
                    _ => None,
                })
                .collect(),
        })
        .collect()
}

impl Data {
    pub fn load(root: &Path) -> Result<Data, String> {
        let npc = root.join("scripts").join("npc");
        let mut data = Data::default();

        let abilities = Kv::parse_file(&npc.join("npc_abilities.txt"))?;
        if let Some(block) = abilities.block("DOTAAbilities") {
            for (name, kv) in block.blocks() {
                data.abilities
                    .insert(name.to_string(), AbilityDef::from_kv(name, kv));
            }
        }

        let items = Kv::parse_file(&npc.join("items.txt"))?;
        if let Some(block) = items.block("DOTAAbilities") {
            for (name, kv) in block.blocks() {
                data.items
                    .insert(name.to_string(), ItemDef::from_kv(name, kv));
            }
        }
        let units = Kv::parse_file(&npc.join("npc_units.txt"))?;
        if let Some(block) = units.block("DOTAUnits") {
            for (name, kv) in block.blocks() {
                data.units
                    .insert(name.to_string(), UnitDef::from_kv(name, kv));
            }
        }

        let dir = npc.join("heroes");
        let base_file = Kv::parse_file(&dir.join("npc_dota_hero_base.txt"))?;
        let base = base_file
            .block("DOTAHeroes")
            .and_then(|b| b.block("npc_dota_hero_base"))
            .cloned()
            .unwrap_or_default();
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "txt"))
            .filter(|p| !p.ends_with("npc_dota_hero_base.txt"))
            .collect();
        files.sort();
        for path in files {
            let file = Kv::parse_file(&path)?;
            let Some(block) = file.block("DOTAHeroes") else {
                continue;
            };
            for (name, kv) in block.blocks() {
                if !name.starts_with("npc_dota_hero_") {
                    continue;
                }
                let mut merged = base.clone();
                merged.merge(kv);
                if let Some(defs) = kv.block("AbilityDefinitions") {
                    for (ability_name, ability) in defs.blocks() {
                        data.abilities.insert(
                            ability_name.to_string(),
                            AbilityDef::from_kv(ability_name, ability),
                        );
                    }
                }
                data.heroes.push(HeroDef::from_kv(name, &merged));
            }
        }
        data.heroes.sort_by_key(|h| h.id);
        data.shops = load_shops(root);
        let language = std::env::var("HORIZON_DOTA_LANG").unwrap_or_else(|_| "english".into());
        data.names = load_names(root, &language);
        Ok(data)
    }

    pub fn display_name(&self, internal: &str) -> String {
        let key = internal.to_ascii_lowercase();
        for candidate in [format!("dota_tooltip_ability_{key}"), key.clone()] {
            if let Some(name) = self.names.get(&candidate) {
                return name.clone();
            }
        }
        internal
            .trim_start_matches("npc_dota_hero_")
            .trim_start_matches("item_")
            .split('_')
            .map(|w| {
                let mut c = w.chars();
                c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn shop(&self, name: &str) -> Option<&Shop> {
        self.shops.iter().find(|s| s.name == name)
    }

    pub fn hero(&self, name: &str) -> Option<&HeroDef> {
        self.heroes.iter().find(|h| h.name == name)
    }

    pub fn ability(&self, name: &str) -> Option<&AbilityDef> {
        self.abilities.get(name)
    }

    pub fn item(&self, name: &str) -> Option<&ItemDef> {
        self.items.get(name)
    }

    pub fn unit(&self, name: &str) -> Option<&UnitDef> {
        self.units.get(name)
    }
}

pub fn find_data_dir() -> Option<std::path::PathBuf> {
    let valid = |p: &std::path::PathBuf| p.join("scripts").join("npc").join("items.txt").is_file();
    std::env::var_os("HORIZON_DOTA_DATA")
        .map(std::path::PathBuf::from)
        .filter(valid)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(|p| p.join("dota-game-data")))
                .filter(valid)
        })
        .or_else(dev_data_dir)
}

pub fn dev_data_dir() -> Option<std::path::PathBuf> {
    let candidates = [
        std::env::var_os("HORIZON_DOTA_DATA").map(std::path::PathBuf::from),
        Some(std::path::PathBuf::from("dota-game-data")),
        Some(std::path::PathBuf::from("../dota-game-data")),
        Some(std::path::PathBuf::from("../../../dota-game-data")),
        Some(std::path::PathBuf::from("../../../../dota-game-data")),
        Some(std::path::PathBuf::from(r"C:\Users\root\Desktop\rust\dota-game-data")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|p| p.join("scripts").join("npc").join("items.txt").is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Option<Data> {
        Data::load(&dev_data_dir()?).ok()
    }

    #[test]
    fn levels_clamp() {
        assert_eq!(at(&[18.0, 16.0, 14.0, 12.0], 1), 18.0);
        assert_eq!(at(&[18.0, 16.0, 14.0, 12.0], 4), 12.0);
        assert_eq!(at(&[18.0, 16.0], 4), 16.0);
        assert_eq!(at(&[], 1), 0.0);
    }

    #[test]
    fn axe_from_the_files() {
        let Some(data) = data() else { return };
        let axe = data.hero("npc_dota_hero_axe").unwrap();
        assert_eq!(axe.primary, Attribute::Strength);
        assert_eq!(axe.base[0], 25.0);
        assert!(axe.melee);
        assert_eq!(axe.abilities[0], "axe_berserkers_call");
        assert!(axe.talents.len() >= 8);
        let call = data.ability("axe_berserkers_call").unwrap();
        assert_eq!(call.cooldown, vec![18.0, 16.0, 14.0, 12.0]);
        assert_eq!(call.mana_at(2), 100.0);
        assert_eq!(call.value("radius", 1), 315.0);
        assert_eq!(call.value("duration", 4), 3.0);
        assert!(call.has_behavior("NO_TARGET"));
    }

    #[test]
    fn every_hero_loads_with_abilities() {
        let Some(data) = data() else { return };
        assert!(data.heroes.len() > 100, "{}", data.heroes.len());
        for hero in data.heroes.iter().filter(|h| !h.name.ends_with("target_dummy")) {
            assert!(hero.id > 0, "{}", hero.name);
            assert!(hero.base.iter().sum::<f32>() > 0.0, "{}", hero.name);
            assert!(!hero.abilities.is_empty(), "{}", hero.name);
        }
        let missing: Vec<_> = data
            .heroes
            .iter()
            .flat_map(|h| h.abilities.iter().map(move |a| (h.name.as_str(), a.as_str())))
            .filter(|(_, a)| data.ability(a).is_none())
            .collect();
        assert!(missing.len() < 40, "{missing:?}");
    }

    #[test]
    fn items_and_recipes() {
        let Some(data) = data() else { return };
        let blink = data.item("item_blink").unwrap();
        assert_eq!(blink.cost, 2250);
        assert!(!blink.recipe);
        let recipe = data.item("item_recipe_blade_mail").unwrap();
        assert!(recipe.recipe);
        assert_eq!(recipe.result, "item_blade_mail");
        assert!(data.item("item_tango").unwrap().cost > 0);
    }

    #[test]
    fn names_and_shops_come_from_the_files() {
        let Some(data) = data() else { return };
        assert_eq!(data.display_name("item_blink"), "Blink Dagger");
        assert_eq!(data.display_name("npc_dota_hero_axe"), "Axe");
        assert_eq!(data.display_name("axe_berserkers_call"), "Berserker's Call");
        assert_eq!(data.display_name("item_no_such_thing_xyz"), "No Such Thing Xyz");
        let consumables = data.shop("consumables").unwrap();
        assert!(consumables.items.contains(&"item_tango".to_string()));
        assert!(data.shops.len() >= 6, "{}", data.shops.len());
        for shop in &data.shops {
            for item in &shop.items {
                assert!(data.item(item).is_some(), "{} lists unknown {item}", shop.name);
            }
        }
    }

    #[test]
    fn costs_and_cooldowns_inside_ability_values_are_found() {
        let Some(data) = data() else { return };
        let bolt = data.ability("sven_storm_bolt").unwrap();
        assert_eq!(bolt.cooldown, vec![21.0, 18.0, 15.0, 12.0]);
        assert_eq!(bolt.mana_at(1), 110.0);
        assert_eq!(bolt.range_at(1), 600.0);
        let mut free = Vec::new();
        for hero in &data.heroes {
            for name in &hero.abilities {
                let Some(a) = data.ability(name) else { continue };
                if a.passive() || a.hidden() || a.has_behavior("TOGGLE") {
                    continue;
                }
                if a.cooldown.is_empty() && a.mana_cost.is_empty() {
                    free.push(name.clone());
                }
            }
        }
        assert!(free.len() < 25, "{} castable abilities with neither cost nor cooldown: {free:?}", free.len());
    }

    #[test]
    fn creeps() {
        let Some(data) = data() else { return };
        let melee = data.unit("npc_dota_creep_goodguys_melee").unwrap();
        assert!(melee.melee);
        assert!(melee.bounty_gold.0 > 0.0);
        assert!(melee.health > 0.0);
    }
}
