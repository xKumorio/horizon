use crate::combat::{FIRST_UNIT_ID, UNIT_METERS, UnitId, distance_xz};
use crate::data::HeroDef;
use crate::kv::Value;
use crate::map::Lane;
use crate::sim::{DT, Intent, Match, PlayerId, Team};

pub const BOT_FIRST_ID: PlayerId = 100;
const RETREAT_BELOW: f32 = 0.28;
const RESUME_ABOVE: f32 = 0.85;
const SHOP_NEAR: f32 = 32.0;
const FOLLOW_GAP: f32 = 7.0;
const CAST_REACH: f32 = 14.0;
const PUSH_AFTER_TICKS: u64 = 15 * 60 * 30;

#[derive(Clone, Debug)]
pub struct BotState {
    pub lane: Lane,
    pub build: Vec<String>,
    pub loadout: Vec<String>,
    pub next_item: usize,
    pub retreating: bool,
    pub path_index: usize,
    pub target: Option<UnitId>,
}

pub fn ability_build(def: &HeroDef) -> Vec<String> {
    let Some(block) = def.raw.block("Bot").and_then(|b| b.block("Build")) else {
        return Vec::new();
    };
    let mut steps: Vec<(u32, String)> = block
        .entries
        .iter()
        .filter_map(|(k, v)| match v {
            Value::Str(s) if !s.is_empty() => k.parse().ok().map(|n| (n, s.clone())),
            _ => None,
        })
        .collect();
    steps.sort_by_key(|(n, _)| *n);
    steps.into_iter().map(|(_, s)| s).collect()
}

pub fn item_loadout(def: &HeroDef) -> Vec<String> {
    let Some(block) = def.raw.block("Bot").and_then(|b| b.block("Loadout")) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for (name, value) in &block.entries {
        let Value::Str(flags) = value else { continue };
        if name.starts_with("item_recipe_") || out.contains(name) && !flags.contains("CONSUMABLE") {
            continue;
        }
        out.push(name.clone());
    }
    out
}

impl Match {
    pub fn add_bot(&mut self, player: PlayerId, team: Team, hero: &str, lane: Lane, pos: [f32; 3]) -> Result<(), String> {
        self.add_hero(player, team, hero, pos)?;
        let def = self.data.hero(hero).cloned().ok_or("no hero")?;
        let state = BotState {
            lane,
            build: ability_build(&def),
            loadout: item_loadout(&def),
            next_item: 0,
            retreating: false,
            path_index: 1,
            target: None,
        };
        if let Some(h) = self.heroes.iter_mut().find(|h| h.player == player) {
            h.bot = Some(state);
        }
        Ok(())
    }

    pub fn bot_ids(&self) -> Vec<PlayerId> {
        self.heroes.iter().filter(|h| h.bot.is_some()).map(|h| h.player).collect()
    }

    fn push_lane(&self, team: Team, own: Lane) -> Lane {
        if self.tick < PUSH_AFTER_TICKS {
            return own;
        }
        let foe = if team == Team::Radiant { Team::Dire } else { Team::Radiant };
        let mut best = (usize::MAX, own);
        for lane in Lane::ALL {
            let tag = match lane {
                Lane::Top => "_top",
                Lane::Mid => "_mid",
                Lane::Bot => "_bot",
            };
            let left = self
                .units
                .iter()
                .filter(|u| u.alive && u.team == foe && u.kind.is_building() && u.key.ends_with(tag) && !u.key.contains("_tower4_"))
                .count();
            if left > 0 && left < best.0 {
                best = (left, lane);
            }
        }
        best.1
    }

    fn bot_cast(&mut self, player: PlayerId, target: UnitId) {
        if target >= FIRST_UNIT_ID {
            return;
        }
        let (Some(me), Some(them)) = (self.combatant(player), self.combatant(target)) else { return };
        let dx = them.pos[0] - me.pos[0];
        let dy = them.pos[1] - me.pos[1];
        let dz = them.pos[2] - me.pos[2];
        let len = (dx * dx + dy * dy + dz * dz).sqrt().max(0.001);
        if len > CAST_REACH {
            return;
        }
        let aim = crate::sim::Aim::Direction([dx / len, dy / len, dz / len]);
        for slot in (0..crate::sim::ABILITY_SLOTS).rev() {
            if self.submit(player, Intent::Cast { slot, aim }).is_ok() {
                break;
            }
        }
    }

    fn fountain_of(&self, team: Team) -> Option<[f32; 3]> {
        self.spawn_positions.get(&team).copied()
    }

    fn leading_creep(&self, lane: Lane, team: Team) -> Option<[f32; 3]> {
        self.units
            .iter()
            .filter(|u| u.alive && u.team == team && u.lane == Some(lane))
            .max_by_key(|u| u.path_index)
            .map(|u| u.pos)
    }

    fn nearest_foe(&self, player: PlayerId, reach: f32) -> Option<UnitId> {
        let me = self.combatant(player)?;
        let mut best: Option<(u32, f32, UnitId)> = None;
        for c in self.all_combatants() {
            if !c.hostile_to(&me) || c.neutral || !c.alive || c.id == player {
                continue;
            }
            if c.id >= FIRST_UNIT_ID && self.unit(c.id).is_some_and(|u| u.kind == crate::ai::UnitKind::Courier) {
                continue;
            }
            if c.id >= FIRST_UNIT_ID && self.unit(c.id).is_some_and(|u| u.invulnerable) {
                continue;
            }
            let d = distance_xz(me.pos, c.pos) - c.radius;
            if d > reach {
                continue;
            }
            let building = c.id >= FIRST_UNIT_ID && self.unit(c.id).is_some_and(|u| u.kind.is_building());
            let priority = if c.id < FIRST_UNIT_ID {
                1
            } else if building {
                2
            } else {
                0
            };
            if best.is_none_or(|b| (priority, d) < (b.0, b.1)) {
                best = Some((priority, d, c.id));
            }
        }
        best.map(|b| b.2)
    }

    fn move_hero(&mut self, player: PlayerId, goal: [f32; 3], stop: f32) -> bool {
        let speed = self.hero(player).map_or(300.0, |h| h.stats(&self.rules).move_speed) * UNIT_METERS * self.speed_factor(player);
        let Some(h) = self.heroes.iter_mut().find(|h| h.player == player) else { return true };
        let dx = goal[0] - h.pos[0];
        let dz = goal[2] - h.pos[2];
        let dist = (dx * dx + dz * dz).sqrt();
        if dist <= stop + 0.05 {
            return true;
        }
        let step = (speed * DT).min(dist - stop).max(0.0);
        h.pos[0] += dx / dist * step;
        h.pos[2] += dz / dist * step;
        h.pos[1] = goal[1];
        false
    }

    fn bot_spend_points(&mut self, player: PlayerId) {
        loop {
            let Some(h) = self.hero(player) else { return };
            if h.skill_points == 0 {
                return;
            }
            let Some(bot) = h.bot.as_ref() else { return };
            let learned: u32 = h.ability_levels.iter().sum();
            let preferred = bot
                .build
                .get(learned as usize)
                .and_then(|name| h.def.abilities.iter().position(|a| a == name));
            let mut slots: Vec<usize> = preferred.into_iter().collect();
            slots.extend((0..h.def.abilities.len()).rev());
            let mut done = false;
            for slot in slots {
                if self.submit(player, Intent::LevelUp(slot)).is_ok() {
                    done = true;
                    break;
                }
            }
            if !done {
                return;
            }
        }
    }

    fn bot_shop(&mut self, player: PlayerId) {
        let Some(h) = self.hero(player) else { return };
        let Some(fountain) = self.fountain_of(h.team) else { return };
        if distance_xz(h.pos, fountain) > SHOP_NEAR {
            return;
        }
        loop {
            let Some(h) = self.hero(player) else { return };
            let Some(bot) = h.bot.as_ref() else { return };
            let Some(item) = bot.loadout.get(bot.next_item).cloned() else { return };
            let Some(cost) = self.item_cost(&item) else {
                self.advance_item(player);
                continue;
            };
            let (net, _) = self.combine_plan(self.heroes.iter().position(|x| x.player == player).unwrap_or(0), &item);
            if h.gold < net.max(0) {
                return;
            }
            let _ = cost;
            match self.submit(player, Intent::Buy { item, at_shop: true }) {
                Ok(()) => self.advance_item(player),
                Err(crate::sim::Reject::InventoryFull) => return,
                Err(_) => self.advance_item(player),
            }
        }
    }

    fn advance_item(&mut self, player: PlayerId) {
        if let Some(bot) = self.heroes.iter_mut().find(|h| h.player == player).and_then(|h| h.bot.as_mut()) {
            bot.next_item += 1;
        }
    }

    pub(crate) fn step_bots(&mut self) {
        for player in self.bot_ids() {
            let Some(h) = self.hero(player) else { continue };
            if !h.alive || self.stunned(player) {
                continue;
            }
            let team = h.team;
            let max = h.stats(&self.rules).max_hp;
            let fraction = h.hp / max.max(1.0);
            let Some(bot) = h.bot.clone() else { continue };
            self.bot_spend_points(player);
            self.bot_shop(player);
            let mut retreating = bot.retreating;
            if fraction < RETREAT_BELOW {
                retreating = true;
            } else if fraction > RESUME_ABOVE {
                retreating = false;
            }
            if let Some(b) = self.heroes.iter_mut().find(|x| x.player == player).and_then(|x| x.bot.as_mut()) {
                b.retreating = retreating;
            }
            if retreating {
                if let Some(home) = self.fountain_of(team) {
                    self.move_hero(player, home, 3.0);
                    if let Some(hh) = self.heroes.iter_mut().find(|x| x.player == player)
                        && distance_xz(hh.pos, home) < SHOP_NEAR
                    {
                        hh.hp = (hh.hp + max * 0.06 * DT).min(max);
                    }
                }
                continue;
            }
            let acquire = self.hero(player).map_or(0.0, |x| x.def.raw.f32_or("AttackAcquisitionRange", 800.0)) * UNIT_METERS;
            let target = self.taunted_by(player).or_else(|| self.nearest_foe(player, acquire));
            if let Some(b) = self.heroes.iter_mut().find(|x| x.player == player).and_then(|x| x.bot.as_mut()) {
                b.target = target;
            }
            if let Some(t) = target {
                let Some(c) = self.combatant(t) else { continue };
                let range = self.hero(player).map_or(1.5, |x| x.def.attack_range * UNIT_METERS) + c.radius;
                let here = self.hero(player).map_or([0.0; 3], |x| x.pos);
                if distance_xz(here, c.pos) <= range + 0.4 {
                    self.bot_cast(player, t);
                    let _ = self.attack(player, t);
                } else {
                    self.move_hero(player, c.pos, range * 0.85);
                }
                continue;
            }
            let lane = self.push_lane(team, bot.lane);
            if let Some(lead) = self.leading_creep(lane, team) {
                self.move_hero(player, lead, FOLLOW_GAP);
                continue;
            }
            let path = self.lanes.get(&(lane, team)).cloned().unwrap_or_default();
            if let Some(goal) = path.get(bot.path_index).copied() {
                if self.move_hero(player, goal, 2.0)
                    && let Some(b) = self.heroes.iter_mut().find(|x| x.player == player).and_then(|x| x.bot.as_mut())
                {
                    b.path_index += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Data, dev_data_dir};
    use crate::map::{MapData, find_map_file};
    use crate::sim::{Rules, TICK_RATE};
    use std::sync::Arc;

    fn world() -> Option<Match> {
        let data = Arc::new(Data::load(&dev_data_dir()?).ok()?);
        let map = MapData::load(&find_map_file()?).ok()?;
        let mut m = Match::new(data, Rules::default());
        m.setup_map(&map);
        Some(m)
    }

    fn run(m: &mut Match, seconds: f32) {
        for _ in 0..(seconds * TICK_RATE as f32) as u32 {
            m.step();
        }
    }

    fn start(m: &Match, team: Team) -> [f32; 3] {
        m.spawn_positions[&team]
    }

    #[test]
    fn the_data_gives_a_bot_its_ability_order_and_shopping_list() {
        let Some(m) = world() else { return };
        let axe = m.data.hero("npc_dota_hero_axe").unwrap();
        let build = ability_build(axe);
        assert_eq!(build[0], "axe_counter_helix");
        assert!(build.iter().filter(|a| a.as_str() == "axe_culling_blade").count() >= 3);
        let items = item_loadout(axe);
        assert!(items.contains(&"item_tango".to_string()));
        assert!(items.contains(&"item_blink".to_string()));
        assert!(items.iter().all(|i| !i.starts_with("item_recipe_")));
    }

    #[test]
    fn a_bot_walks_to_its_lane_and_fights_there() {
        let Some(mut m) = world() else { return };
        let pos = start(&m, Team::Radiant);
        m.add_bot(101, Team::Radiant, "npc_dota_hero_axe", Lane::Mid, pos).unwrap();
        let before = m.hero(101).unwrap().pos;
        run(&mut m, 40.0);
        let h = m.hero(101).unwrap();
        assert!(crate::combat::distance_xz(before, h.pos) > 80.0, "{}", crate::combat::distance_xz(before, h.pos));
        run(&mut m, 60.0);
        let hits = m.units.iter().filter(|u| u.team == Team::Dire && (u.hp < u.max_hp || !u.alive)).count();
        assert!(hits > 0 || m.hero(101).unwrap().last_hits > 0, "nothing was hurt");
    }

    #[test]
    fn a_bot_spends_its_points_by_the_data_order() {
        let Some(mut m) = world() else { return };
        let pos = start(&m, Team::Radiant);
        m.add_bot(101, Team::Radiant, "npc_dota_hero_axe", Lane::Mid, pos).unwrap();
        run(&mut m, 0.2);
        let h = m.hero(101).unwrap();
        let helix = h.def.abilities.iter().position(|a| a == "axe_counter_helix").unwrap();
        assert_eq!(h.ability_levels[helix], 1);
        assert_eq!(h.skill_points, 0);
    }

    #[test]
    fn a_rich_bot_at_the_fountain_buys_its_list_and_stops_when_the_bag_is_full() {
        let Some(mut m) = world() else { return };
        let pos = start(&m, Team::Radiant);
        m.add_bot(101, Team::Radiant, "npc_dota_hero_axe", Lane::Mid, pos).unwrap();
        m.heroes[0].gold = 100000;
        run(&mut m, 0.2);
        let h = m.hero(101).unwrap();
        assert!(h.owns("item_tango"));
        assert!(h.items.iter().flatten().count() >= 5);
    }

    #[test]
    fn a_hurt_bot_retreats_to_the_fountain_and_heals() {
        let Some(mut m) = world() else { return };
        let pos = start(&m, Team::Radiant);
        m.add_bot(101, Team::Radiant, "npc_dota_hero_axe", Lane::Mid, pos).unwrap();
        run(&mut m, 25.0);
        let max = m.hero(101).unwrap().stats(&m.rules).max_hp;
        m.heroes[0].hp = max * 0.1;
        let far = crate::combat::distance_xz(m.hero(101).unwrap().pos, pos);
        assert!(far > 30.0);
        run(&mut m, 40.0);
        let h = m.hero(101).unwrap();
        assert!(h.bot.as_ref().unwrap().retreating || h.hp > max * 0.5);
        assert!(crate::combat::distance_xz(h.pos, pos) < far);
    }

    #[test]
    fn a_dead_bot_comes_back_at_the_fountain() {
        let Some(mut m) = world() else { return };
        let pos = start(&m, Team::Radiant);
        m.add_bot(101, Team::Radiant, "npc_dota_hero_axe", Lane::Mid, pos).unwrap();
        run(&mut m, 25.0);
        m.damage(101, 99999.0);
        assert!(!m.hero(101).unwrap().alive);
        let wait = m.rules.respawn_time(1);
        run(&mut m, wait + 0.5);
        let h = m.hero(101).unwrap();
        assert!(h.alive);
        assert!(crate::combat::distance_xz(h.pos, pos) < 6.0, "{}", crate::combat::distance_xz(h.pos, pos));
    }

    #[test]
    fn ten_bots_play_ten_minutes_without_going_wrong() {
        let Some(mut m) = world() else { return };
        let names = [
            "npc_dota_hero_axe",
            "npc_dota_hero_sven",
            "npc_dota_hero_lina",
            "npc_dota_hero_crystal_maiden",
            "npc_dota_hero_juggernaut",
        ];
        let lanes = [Lane::Mid, Lane::Top, Lane::Bot, Lane::Bot, Lane::Top];
        for team in [Team::Radiant, Team::Dire] {
            let pos = start(&m, team);
            for i in 0..5 {
                let id = BOT_FIRST_ID + if team == Team::Radiant { 0 } else { 10 } + i as u32;
                m.add_bot(id, team, names[i], lanes[i], pos).unwrap();
            }
        }
        run(&mut m, 600.0);
        assert!(m.heroes.iter().all(|h| h.pos.iter().all(|c| c.is_finite()) && h.hp.is_finite()));
        let levelled = m.heroes.iter().filter(|h| h.level > 1).count();
        assert!(levelled >= 5, "{levelled} heroes gained a level");
        let fights = m.heroes.iter().map(|h| h.kills + h.deaths + h.last_hits).sum::<u32>();
        assert!(fights > 0, "nobody fought");
        assert!(m.heroes.iter().any(|h| h.items.iter().flatten().count() > 0));
        for h in &m.heroes {
            println!(
                "MATCH {} {:?} lvl {} gold {} k/d/lh/dn {}/{}/{}/{} items {} hp {:.0} pos {:?}",
                h.def.name,
                h.team,
                h.level,
                h.gold,
                h.kills,
                h.deaths,
                h.last_hits,
                h.denies,
                h.items.iter().flatten().count(),
                h.hp,
                h.pos
            );
        }
        let dead_buildings = m.units.iter().filter(|u| u.kind.is_building() && !u.alive).count();
        println!("MATCH buildings destroyed {dead_buildings}, creeps alive {}", m.units.iter().filter(|u| u.alive && !u.kind.is_building()).count());
    }

    #[test]
    fn a_ten_bot_match_runs_until_an_ancient_falls() {
        let Some(mut m) = world() else { return };
        let names = [
            "npc_dota_hero_axe",
            "npc_dota_hero_sven",
            "npc_dota_hero_lina",
            "npc_dota_hero_crystal_maiden",
            "npc_dota_hero_juggernaut",
        ];
        let lanes = [Lane::Mid, Lane::Top, Lane::Bot, Lane::Bot, Lane::Top];
        for team in [Team::Radiant, Team::Dire] {
            let pos = start(&m, team);
            for i in 0..5 {
                let id = BOT_FIRST_ID + if team == Team::Radiant { 0 } else { 10 } + i as u32;
                m.add_bot(id, team, names[i], lanes[i], pos).unwrap();
            }
        }
        let mut minutes = 0;
        while m.winner.is_none() && minutes < 120 {
            run(&mut m, 60.0);
            minutes += 1;
        }
        let dead = m.units.iter().filter(|u| u.kind.is_building() && !u.alive).count();
        println!("LONG minutes {minutes} winner {:?} buildings down {dead}", m.winner);
        for h in &m.heroes {
            println!("LONG {} {:?} lvl {} k/d/lh {}/{}/{}", h.def.name, h.team, h.level, h.kills, h.deaths, h.last_hits);
        }
        assert!(m.winner.is_some(), "no winner in {minutes} minutes, {dead} buildings down");
    }
}
