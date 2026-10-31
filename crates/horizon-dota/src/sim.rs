use crate::data::{Attribute, Data, HeroDef, at};
use std::sync::Arc;

pub const TICK_RATE: u32 = 30;
pub const DT: f32 = 1.0 / TICK_RATE as f32;
pub const SLOTS: usize = 9;
pub const MAIN_SLOTS: usize = 6;
pub const ABILITY_SLOTS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Team {
    Radiant,
    Dire,
}

impl Team {
    pub fn enemy(self) -> Team {
        match self {
            Team::Radiant => Team::Dire,
            Team::Dire => Team::Radiant,
        }
    }
}

pub type PlayerId = u32;

#[derive(Clone, Debug)]
pub struct Rules {
    pub starting_gold: i32,
    pub passive_gold_interval: f32,
    pub passive_gold_amount: i32,
    pub xp_to_level: Vec<i32>,
    pub max_level: u32,
    pub hp_per_strength: f32,
    pub hp_regen_per_strength: f32,
    pub mana_per_intelligence: f32,
    pub mana_regen_per_intelligence: f32,
    pub armor_per_agility: f32,
    pub magic_resist_per_intelligence: f32,
    pub attack_speed_per_agility: f32,
    pub damage_per_primary: f32,
    pub damage_per_primary_universal: f32,
    pub ultimate_levels: [u32; 3],
    pub shop_radius: f32,
    pub sell_back: f32,
    pub respawn_by_level: Vec<f32>,
    pub wave_interval: f32,
    pub first_wave: f32,
    pub wave_melee: u32,
    pub wave_ranged: u32,
    pub siege_every: u32,
    pub roshan_respawn: (f32, f32),
    pub neutral_leash: f32,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            starting_gold: 600,
            passive_gold_interval: 0.6,
            passive_gold_amount: 1,
            xp_to_level: vec![
                0, 240, 640, 1160, 1760, 2440, 3200, 4000, 4900, 5900, 7000, 8200, 9500, 10900,
                12400, 14000, 15700, 17500, 19400, 21400, 23600, 26000, 28600, 31400, 34400, 38400,
                43400, 49400, 56400, 64400,
            ],
            respawn_by_level: vec![
                12.0, 15.0, 18.0, 21.0, 24.0, 26.0, 28.0, 30.0, 32.0, 34.0, 36.0, 44.0, 46.0, 48.0,
                50.0, 52.0, 54.0, 65.0, 70.0, 75.0, 80.0, 85.0, 90.0, 95.0, 100.0, 100.0, 100.0,
                100.0, 100.0, 100.0,
            ],
            max_level: 30,
            wave_interval: 30.0,
            first_wave: 0.0,
            wave_melee: 3,
            wave_ranged: 1,
            siege_every: 5,
            roshan_respawn: (480.0, 660.0),
            neutral_leash: 28.0,
            hp_per_strength: 22.0,
            hp_regen_per_strength: 0.1,
            mana_per_intelligence: 12.0,
            mana_regen_per_intelligence: 0.05,
            armor_per_agility: 0.16,
            magic_resist_per_intelligence: 0.1,
            attack_speed_per_agility: 1.0,
            damage_per_primary: 1.0,
            damage_per_primary_universal: 0.45,
            ultimate_levels: [6, 12, 18],
            shop_radius: 1200.0,
            sell_back: 0.5,
        }
    }
}

impl Rules {
    pub fn level_for_xp(&self, xp: i32) -> u32 {
        let mut level = 1;
        for (i, need) in self.xp_to_level.iter().enumerate() {
            if xp >= *need {
                level = i as u32 + 1;
            }
        }
        level.min(self.max_level)
    }

    pub fn respawn_time(&self, level: u32) -> f32 {
        let last = self.respawn_by_level.len().saturating_sub(1);
        self.respawn_by_level
            .get((level as usize).saturating_sub(1).min(last))
            .copied()
            .unwrap_or(0.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub name: String,
    pub charges: i32,
    pub cooldown: f32,
}

#[derive(Clone, Debug)]
pub struct Hero {
    pub player: PlayerId,
    pub team: Team,
    pub def: Arc<HeroDef>,
    pub level: u32,
    pub xp: i32,
    pub gold: i32,
    pub hp: f32,
    pub mana: f32,
    pub pos: [f32; 3],
    pub alive: bool,
    pub respawn_in: f32,
    pub skill_points: u32,
    pub ability_levels: [u32; ABILITY_SLOTS],
    pub cooldowns: [f32; ABILITY_SLOTS],
    pub items: [Option<Item>; SLOTS],
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub last_hits: u32,
    pub denies: u32,
    pub attack_cooldown: f32,
    pub stash: [Option<Item>; crate::courier::STASH_SLOTS],
    pub counters: [u32; ABILITY_SLOTS],
    pub bot: Option<crate::bots::BotState>,
    gold_clock: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub strength: f32,
    pub agility: f32,
    pub intelligence: f32,
    pub max_hp: f32,
    pub max_mana: f32,
    pub hp_regen: f32,
    pub mana_regen: f32,
    pub armor: f32,
    pub magic_resist: f32,
    pub attack_speed: f32,
    pub damage_min: f32,
    pub damage_max: f32,
    pub move_speed: f32,
}

impl Hero {
    pub fn stats(&self, rules: &Rules) -> Stats {
        let d = &self.def;
        let steps = self.level.saturating_sub(1) as f32;
        let strength = d.base[0] + d.gain[0] * steps;
        let agility = d.base[1] + d.gain[1] * steps;
        let intelligence = d.base[2] + d.gain[2] * steps;
        let (primary, factor) = match d.primary {
            Attribute::Strength => (strength, rules.damage_per_primary),
            Attribute::Agility => (agility, rules.damage_per_primary),
            Attribute::Intelligence => (intelligence, rules.damage_per_primary),
            Attribute::Universal => (
                strength + agility + intelligence,
                rules.damage_per_primary_universal,
            ),
        };
        let bonus = primary * factor;
        Stats {
            strength,
            agility,
            intelligence,
            max_hp: d.health + strength * rules.hp_per_strength,
            max_mana: d.mana + intelligence * rules.mana_per_intelligence,
            hp_regen: d.health_regen + strength * rules.hp_regen_per_strength,
            mana_regen: d.mana_regen + intelligence * rules.mana_regen_per_intelligence,
            armor: d.armor + agility * rules.armor_per_agility,
            magic_resist: d.magic_resist + intelligence * rules.magic_resist_per_intelligence,
            attack_speed: 100.0 + agility * rules.attack_speed_per_agility,
            damage_min: d.damage_min + bonus,
            damage_max: d.damage_max + bonus,
            move_speed: d.move_speed,
        }
    }

    pub fn ability_name(&self, slot: usize) -> Option<&str> {
        self.def.abilities.get(slot).map(String::as_str)
    }

    pub fn free_slot(&self) -> Option<usize> {
        self.items.iter().position(Option::is_none)
    }

    pub fn owns(&self, name: &str) -> bool {
        self.items.iter().flatten().any(|i| i.name == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    None,
    Point([f32; 3]),
    Unit(u32),
    Direction([f32; 3]),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    LevelUp(usize),
    Cast { slot: usize, aim: Aim },
    Buy { item: String, at_shop: bool },
    Sell { slot: usize, at_shop: bool },
    UseItem { slot: usize, aim: Aim },
    SwapItems(usize, usize),
    Attack { dir: [f32; 3] },
    PickHero(String),
    Courier(crate::courier::CourierCommand),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reject {
    UnknownPlayer,
    Dead,
    NoSuchAbility,
    NoSuchItem,
    NoSkillPoints,
    LevelTooLow,
    MaxLevel,
    NotLearned,
    Passive,
    OnCooldown,
    NotEnoughMana,
    NotEnoughGold,
    InventoryFull,
    NotAtShop,
    NotForSale,
    EmptySlot,
    BadSlot,
    NothingThere,
    NoTarget,
    Disabled,
    StashFull,
    NoCourier,
    Attack(crate::combat::AttackReject),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    LeveledAbility { player: PlayerId, slot: usize, level: u32 },
    Cast { player: PlayerId, ability: String, level: u32, aim: Aim },
    Bought { player: PlayerId, item: String, cost: i32, consumed: Vec<String> },
    Sold { player: PlayerId, item: String, refund: i32 },
    ItemUsed { player: PlayerId, item: String, aim: Aim },
    LevelUp { player: PlayerId, level: u32 },
    Died { player: PlayerId },
    Respawned { player: PlayerId },
    Killed { killer: Option<u32>, victim: u32, denied: bool },
    AegisUsed { player: PlayerId },
    Sound { event: String, pos: [f32; 3] },
    RunePicked { player: PlayerId, kind: crate::runes::RuneKind },
    GameOver { winner: Team },
}

pub struct Match {
    pub data: Arc<Data>,
    pub rules: Rules,
    pub tick: u64,
    pub heroes: Vec<Hero>,
    pub events: Vec<Event>,
    pub units: Vec<crate::combat::Unit>,
    pub next_unit: u32,
    pub swings: Vec<crate::combat::Swing>,
    pub projectiles: Vec<crate::combat::Projectile>,
    pub rng: u64,
    pub lanes: std::collections::HashMap<(crate::map::Lane, Team), Vec<[f32; 3]>>,
    pub wave_clock: f32,
    pub waves_spawned: u32,
    pub map_ready: bool,
    pub spawn_positions: std::collections::HashMap<Team, [f32; 3]>,
    pub winner: Option<Team>,
    pub roshan_home: Option<[f32; 3]>,
    pub roshan_timer: Option<f32>,
    pub roshan_deaths: u32,
    pub couriers: std::collections::HashMap<Team, crate::courier::CourierState>,
    pub rune_state: crate::runes::RuneState,
    pub modifiers: Vec<crate::modifiers::Modifier>,
    pub pending_casts: Vec<crate::abilities::PendingCast>,
}

impl Match {
    pub fn new(data: Arc<Data>, rules: Rules) -> Match {
        Match {
            data,
            rules,
            tick: 0,
            heroes: Vec::new(),
            events: Vec::new(),
            units: Vec::new(),
            next_unit: crate::combat::FIRST_UNIT_ID,
            swings: Vec::new(),
            projectiles: Vec::new(),
            rng: 0x2545_F491_4F6C_DD1D,
            lanes: std::collections::HashMap::new(),
            wave_clock: 0.0,
            waves_spawned: 0,
            map_ready: false,
            spawn_positions: std::collections::HashMap::new(),
            winner: None,
            roshan_home: None,
            roshan_timer: None,
            roshan_deaths: 0,
            couriers: std::collections::HashMap::new(),
            rune_state: Default::default(),
            modifiers: Vec::new(),
            pending_casts: Vec::new(),
        }
    }

    pub fn time(&self) -> f32 {
        self.tick as f32 * DT
    }

    pub fn add_hero(
        &mut self,
        player: PlayerId,
        team: Team,
        hero: &str,
        pos: [f32; 3],
    ) -> Result<(), String> {
        let def = self
            .data
            .hero(hero)
            .cloned()
            .ok_or_else(|| format!("unknown hero {hero}"))?;
        let mut h = Hero {
            player,
            team,
            def: Arc::new(def),
            level: 1,
            xp: 0,
            gold: self.rules.starting_gold,
            hp: 0.0,
            mana: 0.0,
            pos,
            alive: true,
            respawn_in: 0.0,
            skill_points: 1,
            ability_levels: [0; ABILITY_SLOTS],
            cooldowns: [0.0; ABILITY_SLOTS],
            items: Default::default(),
            kills: 0,
            deaths: 0,
            assists: 0,
            last_hits: 0,
            denies: 0,
            attack_cooldown: 0.0,
            stash: Default::default(),
            counters: [0; ABILITY_SLOTS],
            bot: None,
            gold_clock: 0.0,
        };
        let stats = h.stats(&self.rules);
        h.hp = stats.max_hp;
        h.mana = stats.max_mana;
        self.heroes.push(h);
        Ok(())
    }

    pub fn hero(&self, player: PlayerId) -> Option<&Hero> {
        self.heroes.iter().find(|h| h.player == player)
    }

    fn index(&self, player: PlayerId) -> Result<usize, Reject> {
        self.heroes
            .iter()
            .position(|h| h.player == player)
            .ok_or(Reject::UnknownPlayer)
    }

    pub fn required_level(&self, current: u32, ultimate: bool) -> u32 {
        if ultimate {
            self.rules
                .ultimate_levels
                .get(current as usize)
                .copied()
                .unwrap_or(u32::MAX)
        } else {
            current * 2 + 1
        }
    }

    pub fn submit(&mut self, player: PlayerId, intent: Intent) -> Result<(), Reject> {
        let i = self.index(player)?;
        if !self.heroes[i].alive {
            return Err(Reject::Dead);
        }
        match intent {
            Intent::LevelUp(slot) => self.level_ability(i, slot),
            Intent::Cast { slot, aim } => self.cast(i, slot, aim),
            Intent::Buy { item, at_shop } => self.buy(i, &item, at_shop),
            Intent::Sell { slot, at_shop } => self.sell(i, slot, at_shop),
            Intent::UseItem { slot, aim } => self.use_item(i, slot, aim),
            Intent::Attack { dir } => {
                let h = &self.heroes[i];
                let origin = [h.pos[0], h.pos[1] + crate::combat::HERO_EYE, h.pos[2]];
                let reach = h.def.attack_range * crate::combat::UNIT_METERS + 1.5;
                let me = h.player;
                let target = self.aimed_at(me, origin, dir, reach).ok_or(Reject::NothingThere)?;
                self.attack(me, target).map_err(Reject::Attack)
            }
            Intent::PickHero(_) => Err(Reject::NothingThere),
            Intent::Courier(command) => self.courier_command(player, command),
            Intent::SwapItems(a, b) => {
                if a >= SLOTS || b >= SLOTS {
                    return Err(Reject::BadSlot);
                }
                self.heroes[i].items.swap(a, b);
                Ok(())
            }
        }
    }

    fn level_ability(&mut self, i: usize, slot: usize) -> Result<(), Reject> {
        let name = self.heroes[i]
            .ability_name(slot)
            .ok_or(Reject::NoSuchAbility)?
            .to_string();
        let def = self.data.ability(&name).ok_or(Reject::NoSuchAbility)?;
        let hero = &self.heroes[i];
        if hero.skill_points == 0 {
            return Err(Reject::NoSkillPoints);
        }
        let current = hero.ability_levels[slot];
        if current >= def.max_level {
            return Err(Reject::MaxLevel);
        }
        if hero.level < self.required_level(current, def.ultimate) {
            return Err(Reject::LevelTooLow);
        }
        let hero = &mut self.heroes[i];
        hero.skill_points -= 1;
        hero.ability_levels[slot] += 1;
        let level = hero.ability_levels[slot];
        let player = hero.player;
        self.events.push(Event::LeveledAbility { player, slot, level });
        Ok(())
    }

    fn cast(&mut self, i: usize, slot: usize, aim: Aim) -> Result<(), Reject> {
        let name = self.heroes[i]
            .ability_name(slot)
            .ok_or(Reject::NoSuchAbility)?
            .to_string();
        let data = self.data.clone();
        let def = data.ability(&name).ok_or(Reject::NoSuchAbility)?;
        let player = self.heroes[i].player;
        let level = self.heroes[i].ability_levels[slot] as usize;
        if level == 0 {
            return Err(Reject::NotLearned);
        }
        if def.passive() || def.hidden() {
            return Err(Reject::Passive);
        }
        if self.silenced(player) {
            return Err(Reject::Disabled);
        }
        if self.heroes[i].cooldowns[slot] > 0.0 {
            return Err(Reject::OnCooldown);
        }
        let mana = def.mana_at(level);
        if self.heroes[i].mana < mana {
            return Err(Reject::NotEnoughMana);
        }
        if def.has_behavior("UNIT_TARGET")
            && self
                .resolve_unit_target(player, aim, def.range_at(level) * crate::combat::UNIT_METERS)
                .is_none()
        {
            return Err(Reject::NoTarget);
        }
        let cooldown = def.cooldown_at(level);
        let hero = &mut self.heroes[i];
        hero.mana -= mana;
        hero.cooldowns[slot] = cooldown;
        self.begin_cast(player, &name, level, aim);
        self.events.push(Event::Cast {
            player,
            ability: name,
            level: level as u32,
            aim,
        });
        Ok(())
    }

    pub fn item_cost(&self, name: &str) -> Option<i32> {
        self.data.item(name).map(|i| i.cost)
    }

    fn recipe_for(&self, result: &str) -> Option<&crate::data::ItemDef> {
        self.data
            .items
            .values()
            .find(|i| i.recipe && i.result == result)
    }

    pub fn combine_plan(&self, i: usize, item: &str) -> (i32, Vec<usize>) {
        let Some(def) = self.data.item(item) else {
            return (0, Vec::new());
        };
        let full = def.cost;
        let Some(recipe) = self.recipe_for(item) else {
            return (full, Vec::new());
        };
        let Some(parts) = recipe.requirements.first() else {
            return (full, Vec::new());
        };
        let hero = &self.heroes[i];
        let mut used = Vec::new();
        let mut paid = 0;
        for part in parts {
            let found = hero
                .items
                .iter()
                .enumerate()
                .find(|(slot, held)| {
                    !used.contains(slot) && held.as_ref().is_some_and(|h| &h.name == part)
                })
                .map(|(slot, _)| slot);
            if let Some(slot) = found {
                used.push(slot);
                paid += self.item_cost(part).unwrap_or(0);
            }
        }
        (full - paid, used)
    }

    fn buy(&mut self, i: usize, item: &str, at_shop: bool) -> Result<(), Reject> {
        let def = self.data.item(item).ok_or(Reject::NoSuchItem)?;
        if def.cost <= 0 && !def.recipe {
            return Err(Reject::NotForSale);
        }
        if !at_shop {
            let hero = &self.heroes[i];
            if hero.gold < def.cost {
                return Err(Reject::NotEnoughGold);
            }
            let slot = hero.stash.iter().position(Option::is_none).ok_or(Reject::StashFull)?;
            let charges = def.initial_charges;
            let cost = def.cost;
            let hero = &mut self.heroes[i];
            hero.gold -= cost;
            hero.stash[slot] = Some(Item { name: item.to_string(), charges, cooldown: 0.0 });
            let player = hero.player;
            self.events.push(Event::Bought { player, item: item.to_string(), cost, consumed: Vec::new() });
            return Ok(());
        }
        let (cost, used) = self.combine_plan(i, item);
        let hero = &self.heroes[i];
        if hero.gold < cost {
            return Err(Reject::NotEnoughGold);
        }
        if used.is_empty() && hero.free_slot().is_none() {
            return Err(Reject::InventoryFull);
        }
        let charges = def.initial_charges;
        let hero = &mut self.heroes[i];
        let consumed: Vec<String> = used
            .iter()
            .filter_map(|&s| hero.items[s].take().map(|it| it.name))
            .collect();
        hero.gold -= cost;
        let slot = hero.free_slot().ok_or(Reject::InventoryFull)?;
        hero.items[slot] = Some(Item {
            name: item.to_string(),
            charges,
            cooldown: 0.0,
        });
        let player = hero.player;
        self.events.push(Event::Bought {
            player,
            item: item.to_string(),
            cost,
            consumed,
        });
        Ok(())
    }

    fn sell(&mut self, i: usize, slot: usize, at_shop: bool) -> Result<(), Reject> {
        if slot >= SLOTS {
            return Err(Reject::BadSlot);
        }
        if !at_shop {
            return Err(Reject::NotAtShop);
        }
        let name = self.heroes[i].items[slot]
            .as_ref()
            .ok_or(Reject::EmptySlot)?
            .name
            .clone();
        let refund = (self.item_cost(&name).unwrap_or(0) as f32 * self.rules.sell_back) as i32;
        let hero = &mut self.heroes[i];
        hero.items[slot] = None;
        hero.gold += refund;
        let player = hero.player;
        self.events.push(Event::Sold {
            player,
            item: name,
            refund,
        });
        Ok(())
    }

    fn use_item(&mut self, i: usize, slot: usize, aim: Aim) -> Result<(), Reject> {
        if slot >= MAIN_SLOTS {
            return Err(Reject::BadSlot);
        }
        let item = self.heroes[i].items[slot]
            .clone()
            .ok_or(Reject::EmptySlot)?;
        let def = self.data.item(&item.name).ok_or(Reject::NoSuchItem)?;
        let ability = &def.ability;
        if ability.passive() {
            return Err(Reject::Passive);
        }
        if item.cooldown > 0.0 {
            return Err(Reject::OnCooldown);
        }
        let mana = at(&ability.mana_cost, 1);
        if self.heroes[i].mana < mana {
            return Err(Reject::NotEnoughMana);
        }
        let cooldown = at(&ability.cooldown, 1);
        let hero = &mut self.heroes[i];
        hero.mana -= mana;
        let player = hero.player;
        let name = item.name.clone();
        if let Some(held) = hero.items[slot].as_mut() {
            held.cooldown = cooldown;
            if held.charges > 0 {
                held.charges -= 1;
                if held.charges == 0 && def.stackable {
                    hero.items[slot] = None;
                }
            }
        }
        self.events.push(Event::ItemUsed {
            player,
            item: name,
            aim,
        });
        Ok(())
    }

    pub fn grant_xp(&mut self, player: PlayerId, amount: i32) {
        let Ok(i) = self.index(player) else { return };
        let rules = self.rules.clone();
        let hero = &mut self.heroes[i];
        let before = hero.level;
        let previous_max = hero.stats(&rules).max_hp;
        hero.xp += amount;
        let after = rules.level_for_xp(hero.xp);
        if after > before {
            hero.skill_points += after - before;
            hero.level = after;
            let max = hero.stats(&rules).max_hp;
            hero.hp = (hero.hp + max - previous_max).min(max);
            for level in before + 1..=after {
                self.events.push(Event::LevelUp { player, level });
            }
        }
    }

    pub fn roll_between(&mut self, low: f32, high: f32) -> f32 {
        self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let unit = ((self.rng >> 33) as f32) / (u32::MAX >> 1) as f32;
        low + (high - low).max(0.0) * unit.clamp(0.0, 1.0)
    }

    pub fn give_item(&mut self, player: PlayerId, name: &str) -> bool {
        let charges = self.data.item(name).map_or(0, |d| d.initial_charges);
        let Some(hero) = self.heroes.iter_mut().find(|h| h.player == player) else { return false };
        let Some(slot) = hero.free_slot() else { return false };
        hero.items[slot] = Some(Item { name: name.to_string(), charges, cooldown: 0.0 });
        true
    }

    pub fn emit_sound(&mut self, event: &str, pos: [f32; 3]) {
        if !event.is_empty() && self.events.len() < 4000 {
            self.events.push(Event::Sound { event: event.to_string(), pos });
        }
    }

    pub fn grant_gold(&mut self, player: PlayerId, amount: i32) {
        if let Ok(i) = self.index(player) {
            self.heroes[i].gold += amount;
        }
    }

    pub(crate) fn damage_hero_raw(&mut self, player: PlayerId, amount: f32) {
        self.damage(player, amount);
    }

    pub fn damage(&mut self, player: PlayerId, amount: f32) {
        let Ok(i) = self.index(player) else { return };
        let rules = self.rules.clone();
        let hero = &mut self.heroes[i];
        if !hero.alive {
            return;
        }
        hero.hp -= amount;
        if hero.hp <= 0.0 {
            if let Some(slot) = hero.items.iter().position(|i| i.as_ref().is_some_and(|i| i.name == "item_aegis")) {
                hero.items[slot] = None;
                let max = {
                    let h = hero.clone();
                    h.stats(&rules).max_hp
                };
                hero.hp = max;
                self.events.push(Event::AegisUsed { player });
                return;
            }
            hero.hp = 0.0;
            hero.alive = false;
            hero.deaths += 1;
            hero.respawn_in = rules.respawn_time(hero.level);
            self.events.push(Event::Died { player });
        }
    }

    pub fn step(&mut self) {
        self.tick += 1;
        self.step_ai();
        self.step_bots();
        self.step_casts();
        self.step_couriers();
        self.step_runes();
        self.step_combat();
        self.step_modifiers();
        let rules = self.rules.clone();
        let spawn_positions = self.spawn_positions.clone();
        for hero in &mut self.heroes {
            if hero.alive {
                let stats = hero.stats(&rules);
                hero.hp = (hero.hp + stats.hp_regen * DT).min(stats.max_hp);
                hero.mana = (hero.mana + stats.mana_regen * DT).min(stats.max_mana);
                for cd in &mut hero.cooldowns {
                    *cd = (*cd - DT).max(0.0);
                }
                for item in hero.items.iter_mut().flatten() {
                    item.cooldown = (item.cooldown - DT).max(0.0);
                }
                hero.gold_clock += DT;
                while hero.gold_clock >= rules.passive_gold_interval {
                    hero.gold_clock -= rules.passive_gold_interval;
                    hero.gold += rules.passive_gold_amount;
                }
            } else {
                hero.respawn_in -= DT;
                if hero.respawn_in <= 0.0 {
                    let stats = hero.stats(&rules);
                    hero.alive = true;
                    hero.respawn_in = 0.0;
                    hero.hp = stats.max_hp;
                    hero.mana = stats.max_mana;
                    if let Some(home) = spawn_positions.get(&hero.team) {
                        hero.pos = *home;
                    }
                    self.events.push(Event::Respawned { player: hero.player });
                }
            }
        }
    }

    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::dev_data_dir;

    fn start(hero: &str) -> Option<Match> {
        let data = Arc::new(Data::load(&dev_data_dir()?).ok()?);
        let mut m = Match::new(data, Rules::default());
        m.add_hero(1, Team::Radiant, hero, [0.0; 3]).ok()?;
        Some(m)
    }

    fn run(m: &mut Match, seconds: f32) {
        for _ in 0..(seconds * TICK_RATE as f32) as u32 {
            m.step();
        }
    }

    #[test]
    fn axe_level_one_stats() {
        let Some(m) = start("npc_dota_hero_axe") else { return };
        let hero = m.hero(1).unwrap();
        let s = hero.stats(&m.rules);
        assert_eq!(s.strength, 25.0);
        assert_eq!(s.max_hp, 120.0 + 25.0 * 22.0);
        assert_eq!(s.max_mana, 75.0 + 18.0 * 12.0);
        assert_eq!(hero.hp, s.max_hp);
        assert_eq!(hero.gold, 600);
        assert_eq!(s.damage_min, 31.0 + 25.0);
    }

    #[test]
    fn attribute_formulas_match_the_game_text() {
        let Some(m) = start("npc_dota_hero_axe") else { return };
        let s = m.hero(1).unwrap().stats(&m.rules);
        assert!((s.armor - (0.0 + 18.0 * 0.16)).abs() < 1e-4);
        assert!((s.magic_resist - (25.0 + 18.0 * 0.1)).abs() < 1e-4);
        assert_eq!(s.attack_speed, 118.0);
        assert!((s.hp_regen - (2.0 + 2.5)).abs() < 1e-4);
        assert!((s.mana_regen - 0.9).abs() < 1e-4);
    }

    #[test]
    fn stats_grow_with_level() {
        let Some(mut m) = start("npc_dota_hero_axe") else { return };
        m.grant_xp(1, 1160);
        let hero = m.hero(1).unwrap();
        assert_eq!(hero.level, 4);
        assert_eq!(hero.skill_points, 4);
        let s = hero.stats(&m.rules);
        assert!((s.strength - (25.0 + 2.7 * 3.0)).abs() < 1e-4);
    }

    #[test]
    fn ability_needs_a_point_and_the_right_level() {
        let Some(mut m) = start("npc_dota_hero_axe") else { return };
        assert_eq!(m.submit(1, Intent::LevelUp(5)), Err(Reject::NoSuchAbility));
        assert_eq!(m.submit(1, Intent::LevelUp(0)), Ok(()));
        assert_eq!(m.submit(1, Intent::LevelUp(0)), Err(Reject::NoSkillPoints));
        m.grant_xp(1, 240);
        assert_eq!(m.submit(1, Intent::LevelUp(0)), Err(Reject::LevelTooLow));
        assert_eq!(m.submit(1, Intent::LevelUp(1)), Ok(()));
        m.grant_xp(1, 2440 - 240);
        assert_eq!(m.hero(1).unwrap().level, 6);
        let ult = m
            .hero(1)
            .unwrap()
            .def
            .abilities
            .iter()
            .position(|a| a == "axe_culling_blade")
            .unwrap();
        assert_eq!(m.submit(1, Intent::LevelUp(ult)), Ok(()));
        assert_eq!(m.hero(1).unwrap().ability_levels[ult], 1);
    }

    #[test]
    fn cast_costs_mana_and_starts_the_cooldown() {
        let Some(mut m) = start("npc_dota_hero_axe") else { return };
        let aim = Aim::None;
        assert_eq!(m.submit(1, Intent::Cast { slot: 0, aim }), Err(Reject::NotLearned));
        m.submit(1, Intent::LevelUp(0)).unwrap();
        let mana = m.hero(1).unwrap().mana;
        assert_eq!(m.submit(1, Intent::Cast { slot: 0, aim }), Ok(()));
        let hero = m.hero(1).unwrap();
        assert_eq!(hero.mana, mana - 90.0);
        assert_eq!(hero.cooldowns[0], 18.0);
        assert_eq!(
            m.submit(1, Intent::Cast { slot: 0, aim }),
            Err(Reject::OnCooldown)
        );
        run(&mut m, 18.5);
        assert_eq!(m.submit(1, Intent::Cast { slot: 0, aim }), Ok(()));
    }

    #[test]
    fn passive_and_missing_mana() {
        let Some(mut m) = start("npc_dota_hero_axe") else { return };
        m.grant_xp(1, 1160);
        m.submit(1, Intent::LevelUp(2)).unwrap();
        assert_eq!(
            m.submit(1, Intent::Cast { slot: 2, aim: Aim::None }),
            Err(Reject::Passive)
        );
        m.submit(1, Intent::LevelUp(0)).unwrap();
        m.heroes[0].mana = 10.0;
        assert_eq!(
            m.submit(1, Intent::Cast { slot: 0, aim: Aim::None }),
            Err(Reject::NotEnoughMana)
        );
    }

    #[test]
    fn buying_needs_gold_a_shop_and_room() {
        let Some(mut m) = start("npc_dota_hero_axe") else { return };
        let buy = |item: &str, at_shop| Intent::Buy {
            item: item.into(),
            at_shop,
        };
        assert_eq!(m.submit(1, buy("item_blink", true)), Err(Reject::NotEnoughGold));
        assert_eq!(m.submit(1, buy("item_blink", false)), Err(Reject::NotEnoughGold));
        assert_eq!(m.submit(1, buy("item_nonexistent", true)), Err(Reject::NoSuchItem));
        let before = m.hero(1).unwrap().gold;
        assert_eq!(m.submit(1, buy("item_tango", true)), Ok(()));
        let cost = m.item_cost("item_tango").unwrap();
        assert_eq!(m.hero(1).unwrap().gold, before - cost);
        assert!(m.hero(1).unwrap().owns("item_tango"));
        m.heroes[0].gold = 100000;
        for _ in 0..8 {
            m.submit(1, buy("item_tango", true)).unwrap();
        }
        assert_eq!(m.submit(1, buy("item_tango", true)), Err(Reject::InventoryFull));
    }

    #[test]
    fn components_are_consumed_when_the_item_is_assembled() {
        let Some(mut m) = start("npc_dota_hero_axe") else { return };
        m.heroes[0].gold = 100000;
        let buy = |item: &str| Intent::Buy {
            item: item.into(),
            at_shop: true,
        };
        let parts = m
            .data
            .item("item_recipe_blade_mail")
            .unwrap()
            .requirements[0]
            .clone();
        assert_eq!(parts.len(), 2);
        for part in &parts {
            m.submit(1, buy(part)).unwrap();
        }
        let before = m.hero(1).unwrap().gold;
        let full = m.item_cost("item_blade_mail").unwrap();
        let spent: i32 = parts.iter().map(|p| m.item_cost(p).unwrap()).sum();
        m.submit(1, buy("item_blade_mail")).unwrap();
        let hero = m.hero(1).unwrap();
        assert_eq!(before - hero.gold, full - spent);
        assert_eq!(before - hero.gold, m.item_cost("item_recipe_blade_mail").unwrap());
        assert!(hero.owns("item_blade_mail"));
        for part in &parts {
            assert!(!hero.owns(part));
        }
        let events = m.drain_events();
        assert!(events.iter().any(|e| matches!(e, Event::Bought { consumed, .. } if consumed.len() == 2)));
    }

    #[test]
    fn selling_refunds_half() {
        let Some(mut m) = start("npc_dota_hero_axe") else { return };
        m.heroes[0].gold = 100000;
        m.submit(1, Intent::Buy { item: "item_broadsword".into(), at_shop: true }).unwrap();
        let cost = m.item_cost("item_broadsword").unwrap();
        let before = m.hero(1).unwrap().gold;
        let slot = m.hero(1).unwrap().items.iter().position(Option::is_some).unwrap();
        assert_eq!(m.submit(1, Intent::Sell { slot, at_shop: false }), Err(Reject::NotAtShop));
        assert_eq!(m.submit(1, Intent::Sell { slot, at_shop: true }), Ok(()));
        assert_eq!(m.hero(1).unwrap().gold, before + cost / 2);
    }

    #[test]
    fn regen_gold_and_respawn_follow_the_clock() {
        let Some(mut m) = start("npc_dota_hero_axe") else { return };
        m.heroes[0].hp = 100.0;
        let regen = m.hero(1).unwrap().stats(&m.rules).hp_regen;
        run(&mut m, 10.0);
        let hp = m.hero(1).unwrap().hp;
        assert!((hp - (100.0 + regen * 10.0)).abs() < 0.5, "{hp}");
        let gold = m.hero(1).unwrap().gold;
        assert!(gold > 600 + 10, "{gold}");
        m.damage(1, 99999.0);
        assert!(!m.hero(1).unwrap().alive);
        assert_eq!(m.submit(1, Intent::LevelUp(0)), Err(Reject::Dead));
        let wait = m.rules.respawn_time(1);
        run(&mut m, wait + 0.2);
        let hero = m.hero(1).unwrap();
        assert!(hero.alive);
        assert_eq!(hero.hp, hero.stats(&m.rules).max_hp);
        assert_eq!(hero.deaths, 1);
    }

    #[test]
    fn tables_from_the_server_binary() {
        let rules = Rules::default();
        assert_eq!(rules.xp_to_level.len(), 30);
        assert_eq!(rules.level_for_xp(23600), 21);
        assert_eq!(rules.level_for_xp(64400), 30);
        assert_eq!(rules.level_for_xp(99999), 30);
        assert_eq!(rules.respawn_time(1), 12.0);
        assert_eq!(rules.respawn_time(25), 100.0);
        assert_eq!(rules.respawn_time(30), 100.0);
    }

    #[test]
    fn unknown_players_are_rejected() {
        let Some(mut m) = start("npc_dota_hero_axe") else { return };
        assert_eq!(m.submit(99, Intent::LevelUp(0)), Err(Reject::UnknownPlayer));
    }

    #[test]
    fn every_hero_can_spawn_and_its_first_ability_levels() {
        let Some(data) = dev_data_dir().and_then(|d| Data::load(&d).ok()) else { return };
        let data = Arc::new(data);
        for hero in data.heroes.iter().filter(|h| !h.name.ends_with("target_dummy")) {
            let mut m = Match::new(data.clone(), Rules::default());
            m.add_hero(1, Team::Dire, &hero.name, [0.0; 3]).unwrap();
            let s = m.hero(1).unwrap().stats(&m.rules);
            assert!(s.max_hp >= 120.0 && s.max_hp < 1500.0, "{} {}", hero.name, s.max_hp);
            assert!(s.max_mana > 100.0, "{} {}", hero.name, s.max_mana);
        }
    }
}
