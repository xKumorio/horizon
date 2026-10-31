#[allow(unused_imports)]
use crate::combat::{Combatant, FIRST_UNIT_ID, UNIT_METERS, UnitId, distance, distance_xz};
use crate::map::{Lane, MapData};
use crate::sim::{DT, Match, Team};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Creep,
    Siege,
    Tower,
    Barracks,
    Fort,
    Neutral,
    Courier,
}

impl UnitKind {
    pub fn is_building(self) -> bool {
        matches!(self, UnitKind::Tower | UnitKind::Barracks | UnitKind::Fort)
    }
}

pub fn team_prefix(team: Team) -> &'static str {
    match team {
        Team::Radiant => "goodguys",
        Team::Dire => "badguys",
    }
}

pub fn wave_units(team: Team, wave: u32, rules: &crate::sim::Rules) -> Vec<String> {
    let side = team_prefix(team);
    let mut out = Vec::new();
    for _ in 0..rules.wave_melee {
        out.push(format!("npc_dota_creep_{side}_melee"));
    }
    for _ in 0..rules.wave_ranged {
        out.push(format!("npc_dota_creep_{side}_ranged"));
    }
    if rules.siege_every > 0 && wave % rules.siege_every == rules.siege_every - 1 {
        out.push(format!("npc_dota_{side}_siege"));
    }
    out
}

pub fn building_requires(key: &str) -> Vec<String> {
    let good = key.contains("goodguys") || key.starts_with("good_");
    let side = if good { "goodguys" } else { "badguys" };
    let lane = key.rsplit('_').next().unwrap_or("");
    let tower = |tier: u32, lane: &str| format!("dota_{side}_tower{tier}_{lane}");
    if let Some(at) = key.find("_tower") {
        let tier: u32 = key[at + 6..].chars().next().and_then(|c| c.to_digit(10)).unwrap_or(1);
        return match tier {
            2 => vec![tower(1, lane)],
            3 => vec![tower(2, lane)],
            4 => vec![tower(3, "top"), tower(3, "mid"), tower(3, "bot")],
            _ => Vec::new(),
        };
    }
    if key.contains("_rax_") {
        return vec![tower(3, lane)];
    }
    if key.ends_with("_fort") {
        return vec![tower(4, "top"), tower(4, "bot")];
    }
    Vec::new()
}

impl Match {
    pub fn setup_map(&mut self, map: &MapData) {
        self.lanes.clear();
        for team in [Team::Radiant, Team::Dire] {
            for lane in Lane::ALL {
                self.lanes.insert((lane, team), map.lane(lane, team));
            }
            for e in map.buildings(team) {
                let name = if e.unit.is_empty() { e.name.clone() } else { e.unit.clone() };
                let Some(id) = self.spawn_unit(&name, team, e.pos) else { continue };
                let kind = if e.class == "npc_dota_tower" {
                    UnitKind::Tower
                } else if e.class == "npc_dota_barracks" {
                    UnitKind::Barracks
                } else {
                    UnitKind::Fort
                };
                if let Some(u) = self.units.iter_mut().find(|u| u.id == id) {
                    u.kind = kind;
                    u.ai = kind == UnitKind::Tower;
                    if !e.name.is_empty() {
                        u.key = e.name.clone();
                    }
                }
            }
        }
        for team in [Team::Radiant, Team::Dire] {
            if let Some(p) = map.spawn_points(team).first() {
                self.spawn_positions.insert(team, *p);
            }
        }
        if let Some(e) = map.of_class("npc_dota_roshan_spawner").next() {
            self.roshan_home = Some(e.pos);
            self.spawn_roshan();
        }
        let spawners = map
            .entities
            .iter()
            .filter(|e| e.class.starts_with("dota_item_rune_spawner_"))
            .map(|e| crate::runes::RuneSpawner { bounty: e.class.ends_with("_bounty"), pos: e.pos })
            .collect();
        self.setup_runes(spawners);
        self.spawn_couriers();
        self.map_ready = true;
        self.wave_clock = self.rules.first_wave;
        self.refresh_invulnerability();
    }

    pub fn refresh_invulnerability(&mut self) {
        let alive: Vec<String> = self.units.iter().filter(|u| u.alive).map(|u| u.key.clone()).collect();
        for u in &mut self.units {
            if !u.kind.is_building() {
                continue;
            }
            let needs = building_requires(&u.key);
            let blocked = match u.kind {
                UnitKind::Fort => needs.iter().any(|n| alive.contains(n)),
                UnitKind::Tower if u.key.contains("_tower4_") => needs.iter().all(|n| alive.contains(n)),
                _ => needs.iter().any(|n| alive.contains(n)),
            };
            u.invulnerable = blocked;
        }
    }

    pub fn spawn_wave(&mut self) {
        let wave = self.waves_spawned;
        self.waves_spawned += 1;
        for team in [Team::Radiant, Team::Dire] {
            for lane in Lane::ALL {
                let Some(path) = self.lanes.get(&(lane, team)).cloned() else { continue };
                let Some(first) = path.first().copied() else { continue };
                for (n, name) in wave_units(team, wave, &self.rules).into_iter().enumerate() {
                    let offset = [(n % 2) as f32 * 1.2 - 0.6, 0.0, (n / 2) as f32 * -1.4];
                    let pos = [first[0] + offset[0], first[1], first[2] + offset[2]];
                    if let Some(id) = self.spawn_unit(&name, team, pos) {
                        let kind = if name.contains("siege") { UnitKind::Siege } else { UnitKind::Creep };
                        if let Some(u) = self.units.iter_mut().find(|u| u.id == id) {
                            u.kind = kind;
                            u.ai = true;
                            u.lane = Some(lane);
                            u.path = path.clone();
                            u.path_index = 1;
                        }
                    }
                }
            }
        }
    }

    pub fn spawn_roshan(&mut self) -> Option<UnitId> {
        let home = self.roshan_home?;
        let id = self.spawn_unit("npc_dota_roshan", Team::Dire, home)?;
        let u = self.units.iter_mut().find(|u| u.id == id)?;
        u.kind = UnitKind::Neutral;
        u.neutral = true;
        u.ai = true;
        Some(id)
    }

    pub fn roshan(&self) -> Option<&crate::combat::Unit> {
        self.units.iter().find(|u| u.alive && u.kind == UnitKind::Neutral)
    }

    pub(crate) fn on_roshan_killed(&mut self, killer: Option<UnitId>, victim: UnitId) {
        let Some(unit) = self.unit(victim).cloned() else { return };
        let Some(k) = killer.and_then(|k| self.combatant(k)) else {
            self.roshan_timer = Some(self.rules.roshan_respawn.0);
            return;
        };
        let team_heroes: Vec<crate::sim::PlayerId> = self.heroes.iter().filter(|h| h.team == k.team).map(|h| h.player).collect();
        for p in team_heroes {
            let gold = self.roll_between(unit.bounty_gold.0, unit.bounty_gold.1).round() as i32;
            self.grant_gold(p, gold);
        }
        if let Some(id) = killer.filter(|k| *k < FIRST_UNIT_ID) {
            self.give_item(id, "item_aegis");
        }
        self.roshan_deaths += 1;
        let (low, high) = self.rules.roshan_respawn;
        self.roshan_timer = Some(self.roll_between(low, high));
    }

    fn pick_target(&self, id: UnitId) -> Option<UnitId> {
        let u = self.unit(id)?;
        let reach = if u.kind == UnitKind::Tower {
            (u.attack_range + u.radius) * UNIT_METERS + 1.0
        } else {
            u.acquisition_range * UNIT_METERS
        };
        let me = self.combatant(id)?;
        let mut best: Option<(u32, f32, UnitId)> = None;
        for c in self.all_combatants() {
            if !c.hostile_to(&me) || c.neutral || !c.alive || c.id == id {
                continue;
            }
            if c.id >= FIRST_UNIT_ID && self.unit(c.id).is_some_and(|t| t.kind == UnitKind::Courier) {
                continue;
            }
            if c.id >= crate::combat::FIRST_UNIT_ID && self.unit(c.id).is_some_and(|t| t.invulnerable) {
                continue;
            }
            let d = distance_xz(me.pos, c.pos) - c.radius;
            if d > reach {
                continue;
            }
            let priority = if c.id < crate::combat::FIRST_UNIT_ID { 1 } else { 0 };
            let key = (priority, d, c.id);
            if best.is_none_or(|b| (key.0, key.1) < (b.0, b.1)) {
                best = Some(key);
            }
        }
        best.map(|b| b.2)
    }

    pub(crate) fn step_toward(&mut self, id: UnitId, goal: [f32; 3], stop: f32) -> bool {
        let factor = self.speed_factor(id);
        let Some(u) = self.units.iter_mut().find(|u| u.id == id) else { return false };
        let dx = goal[0] - u.pos[0];
        let dz = goal[2] - u.pos[2];
        let dist = (dx * dx + dz * dz).sqrt();
        if dist <= stop + 0.05 {
            return true;
        }
        let step = (u.move_speed * UNIT_METERS * DT * factor).min(dist - stop).max(0.0);
        u.pos[0] += dx / dist * step;
        u.pos[2] += dz / dist * step;
        u.pos[1] = goal[1];
        false
    }

    fn think(&mut self, id: UnitId) {
        let Some(u) = self.unit(id).cloned() else { return };
        if self.stunned(id) {
            return;
        }
        if u.neutral {
            self.think_neutral(id);
            return;
        }
        let movable = !u.kind.is_building();
        let mut target = u.target.filter(|t| {
            self.combatant(*t).is_some_and(|c| {
                let me_team = u.team;
                c.alive
                    && (c.neutral || u.neutral || c.team != me_team)
                    && distance_xz(u.pos, c.pos) <= u.acquisition_range * UNIT_METERS * 1.4 + c.radius
                    && !(c.id >= crate::combat::FIRST_UNIT_ID && self.unit(c.id).is_some_and(|t| t.invulnerable))
            })
        });
        if target.is_none() {
            target = self.pick_target(id);
        }
        if let Some(src) = self.taunted_by(id) {
            target = Some(src);
        }
        if let Some(unit) = self.units.iter_mut().find(|x| x.id == id) {
            unit.target = target;
        }
        if let Some(t) = target {
            let Some(c) = self.combatant(t) else { return };
            let range = (u.attack_range + u.radius) * UNIT_METERS + c.radius;
            if distance_xz(u.pos, c.pos) <= range {
                let _ = self.attack(id, t);
            } else if movable {
                self.step_toward(id, c.pos, range * 0.9);
            }
            return;
        }
        if movable && u.path_index < u.path.len() {
            let goal = u.path[u.path_index];
            if self.step_toward(id, goal, 1.5) {
                if let Some(unit) = self.units.iter_mut().find(|x| x.id == id) {
                    unit.path_index += 1;
                }
            }
        }
    }

    pub(crate) fn step_ai(&mut self) {
        if let Some(left) = self.roshan_timer {
            let left = left - DT;
            if left <= 0.0 {
                self.roshan_timer = None;
                self.spawn_roshan();
            } else {
                self.roshan_timer = Some(left);
            }
        }
        if self.map_ready {
            self.wave_clock -= DT;
            if self.wave_clock <= 0.0 {
                self.spawn_wave();
                self.wave_clock += self.rules.wave_interval;
            }
        }
        let ids: Vec<UnitId> = self.units.iter().filter(|u| u.alive && u.ai).map(|u| u.id).collect();
        for id in ids {
            self.think(id);
        }
    }

    fn think_neutral(&mut self, id: UnitId) {
        let Some(u) = self.unit(id).cloned() else { return };
        if self.stunned(id) {
            return;
        }
        let leash = self.rules.neutral_leash;
        let target = u.aggro.filter(|t| {
            self.combatant(*t)
                .is_some_and(|c| c.alive && distance_xz(u.home, c.pos) < leash)
        });
        if let Some(unit) = self.units.iter_mut().find(|x| x.id == id) {
            unit.target = target;
        }
        match target.and_then(|t| self.combatant(t)) {
            Some(c) => {
                let range = (u.attack_range + u.radius) * UNIT_METERS + c.radius;
                if distance_xz(u.pos, c.pos) <= range {
                    let _ = self.attack(id, c.id);
                } else {
                    self.step_toward(id, c.pos, range * 0.9);
                }
            }
            None => {
                if let Some(unit) = self.units.iter_mut().find(|x| x.id == id) {
                    unit.aggro = None;
                }
                self.step_toward(id, u.home, 0.5);
            }
        }
    }

    pub fn lane_units(&self, lane: Lane, team: Team) -> Vec<&crate::combat::Unit> {
        self.units.iter().filter(|u| u.alive && u.lane == Some(lane) && u.team == team).collect()
    }

    pub fn creeps_by_lane(&self) -> HashMap<(Lane, Team), usize> {
        let mut out = HashMap::new();
        for u in self.units.iter().filter(|u| u.alive && u.lane.is_some()) {
            *out.entry((u.lane.unwrap(), u.team)).or_insert(0) += 1;
        }
        out
    }

    pub fn nearest_enemy(&self, id: UnitId) -> Option<Combatant> {
        self.pick_target(id).and_then(|t| self.combatant(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Data, dev_data_dir};
    use crate::map::find_map_file;
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

    #[test]
    fn the_map_fills_with_buildings_and_closes_the_inner_ones() {
        let Some(m) = world() else { return };
        for team in [Team::Radiant, Team::Dire] {
            let mine: Vec<_> = m.units.iter().filter(|u| u.team == team && u.kind.is_building()).collect();
            assert_eq!(mine.iter().filter(|u| u.kind == UnitKind::Tower).count(), 11, "{team:?}");
            assert_eq!(mine.iter().filter(|u| u.kind == UnitKind::Barracks).count(), 6);
            assert_eq!(mine.iter().filter(|u| u.kind == UnitKind::Fort).count(), 1);
            let t1 = mine.iter().find(|u| u.key.contains("_tower1_mid")).unwrap();
            let t2 = mine.iter().find(|u| u.key.contains("_tower2_mid")).unwrap();
            let fort = mine.iter().find(|u| u.kind == UnitKind::Fort).unwrap();
            assert!(!t1.invulnerable);
            assert!(t2.invulnerable);
            assert!(fort.invulnerable);
            assert!(t1.hp > 1000.0 && t1.armor > 5.0);
        }
    }

    #[test]
    fn the_team_that_destroys_the_ancient_wins() {
        let Some(mut m) = world() else { return };
        let fort = m.units.iter().find(|u| u.kind == UnitKind::Fort && u.team == Team::Dire).unwrap().id;
        assert!(m.unit(fort).unwrap().invulnerable);
        m.deal_damage(None, fort, 99999.0, crate::combat::DamageType::Pure);
        assert!(m.winner.is_none(), "a protected ancient cannot fall");
        m.units.iter_mut().find(|u| u.id == fort).unwrap().invulnerable = false;
        m.deal_damage(Some(1), fort, 99999.0, crate::combat::DamageType::Pure);
        assert_eq!(m.winner, Some(Team::Radiant));
        assert!(m.drain_events().iter().any(|e| matches!(e, crate::sim::Event::GameOver { winner: Team::Radiant })));
    }

    #[test]
    fn roshan_stands_in_his_pit_with_the_numbers_from_the_files() {
        let Some(m) = world() else { return };
        let r = m.roshan().unwrap();
        assert_eq!((r.max_hp, r.armor, r.damage), (6000.0, 30.0, (75.0, 75.0)));
        assert!(r.neutral && r.kind == UnitKind::Neutral);
        assert!(crate::combat::distance(r.pos, m.roshan_home.unwrap()) < 0.1);
    }

    #[test]
    fn roshan_ignores_walkers_but_fights_back_and_goes_home() {
        let Some(mut m) = world() else { return };
        let home = m.roshan_home.unwrap();
        m.add_hero(1, Team::Radiant, "npc_dota_hero_axe", [home[0] + 3.0, home[1], home[2]]).unwrap();
        let hp = m.hero(1).unwrap().hp;
        run(&mut m, 5.0);
        assert!(m.hero(1).unwrap().hp >= hp, "a passing hero is left alone");
        let id = m.roshan().unwrap().id;
        m.deal_damage(Some(1), id, 10.0, crate::combat::DamageType::Pure);
        run(&mut m, 6.0);
        assert!(m.hero(1).unwrap().hp < hp, "the attacked pit boss hits back");
        m.heroes[0].pos = [home[0] + 80.0, home[1], home[2]];
        run(&mut m, 12.0);
        assert!(m.roshan().unwrap().aggro.is_none(), "he gives up the chase");
        assert!(distance_xz(m.roshan().unwrap().pos, home) < 2.0, "and walks back");
    }

    #[test]
    fn killing_roshan_pays_the_whole_team_gives_the_aegis_and_starts_the_respawn() {
        let Some(mut m) = world() else { return };
        let home = m.roshan_home.unwrap();
        m.add_hero(1, Team::Radiant, "npc_dota_hero_axe", [home[0] + 3.0, home[1], home[2]]).unwrap();
        m.add_hero(2, Team::Radiant, "npc_dota_hero_sven", [-200.0, 200.0, -160.0]).unwrap();
        m.add_hero(3, Team::Dire, "npc_dota_hero_pudge", [100.0, 200.0, 120.0]).unwrap();
        let gold: Vec<i32> = [1, 2, 3].iter().map(|p| m.hero(*p).unwrap().gold).collect();
        let id = m.roshan().unwrap().id;
        m.units.iter_mut().find(|u| u.id == id).unwrap().hp = 5.0;
        m.deal_damage(Some(1), id, 100.0, crate::combat::DamageType::Pure);
        assert!(m.roshan().is_none());
        assert!(m.hero(1).unwrap().owns("item_aegis"));
        assert!(!m.hero(2).unwrap().owns("item_aegis"));
        for (p, before) in [(1, gold[0]), (2, gold[1])] {
            let paid = m.hero(p).unwrap().gold - before;
            assert!((200..=290).contains(&paid), "hero {p} got {paid}");
        }
        assert_eq!(m.hero(3).unwrap().gold, gold[2]);
        let timer = m.roshan_timer.unwrap();
        assert!((480.0..=660.0).contains(&timer), "{timer}");
        m.roshan_timer = Some(1.0);
        run(&mut m, 1.5);
        assert!(m.roshan().is_some_and(|r| r.hp == r.max_hp), "he comes back whole");
        assert_eq!(m.roshan_deaths, 1);
    }

    #[test]
    fn the_aegis_buys_one_life_and_is_used_up() {
        let Some(mut m) = world() else { return };
        m.add_hero(1, Team::Radiant, "npc_dota_hero_axe", [0.0, 200.0, 0.0]).unwrap();
        assert!(m.give_item(1, "item_aegis"));
        let max = m.hero(1).unwrap().stats(&m.rules).max_hp;
        m.damage(1, 99999.0);
        assert!(m.hero(1).unwrap().alive);
        assert_eq!(m.hero(1).unwrap().hp, max);
        assert!(!m.hero(1).unwrap().owns("item_aegis"));
        assert!(m.drain_events().iter().any(|e| matches!(e, crate::sim::Event::AegisUsed { player: 1 })));
        m.damage(1, 99999.0);
        assert!(!m.hero(1).unwrap().alive);
    }

    #[test]
    fn lane_creeps_and_bots_leave_roshan_alone() {
        let Some(mut m) = world() else { return };
        let home = m.roshan_home.unwrap();
        let creep = m.spawn_unit("npc_dota_creep_goodguys_melee", Team::Radiant, [home[0] + 2.0, home[1], home[2]]).unwrap();
        m.units.iter_mut().find(|u| u.id == creep).unwrap().ai = true;
        run(&mut m, 3.0);
        let r = m.roshan().unwrap();
        assert_eq!(r.hp, r.max_hp);
    }

    #[test]
    fn tearing_down_a_tower_opens_the_next() {
        let Some(mut m) = world() else { return };
        let (t1, t2) = {
            let find = |n: &str| m.units.iter().find(|u| u.key == n).unwrap().id;
            (find("dota_goodguys_tower1_mid"), find("dota_goodguys_tower2_mid"))
        };
        assert!(m.unit(t2).unwrap().invulnerable);
        m.deal_damage(None, t2, 5000.0, crate::combat::DamageType::Pure);
        assert!(m.unit(t2).unwrap().alive, "an invulnerable building takes no damage");
        m.deal_damage(None, t1, 99999.0, crate::combat::DamageType::Pure);
        m.refresh_invulnerability();
        assert!(!m.unit(t2).unwrap().invulnerable);
    }

    #[test]
    fn a_wave_leaves_every_lane_for_both_teams_on_the_clock() {
        let Some(mut m) = world() else { return };
        run(&mut m, 0.2);
        let first = m.creeps_by_lane();
        let per = (m.rules.wave_melee + m.rules.wave_ranged) as usize;
        for team in [Team::Radiant, Team::Dire] {
            for lane in Lane::ALL {
                assert_eq!(first[&(lane, team)], per, "{lane:?} {team:?}");
            }
        }
        let interval = m.rules.wave_interval;
        run(&mut m, interval);
        assert_eq!(m.waves_spawned, 2);
    }

    #[test]
    fn creeps_follow_the_lane_and_the_two_waves_meet_and_fight() {
        let Some(mut m) = world() else { return };
        run(&mut m, 0.2);
        let before = m.lane_units(Lane::Mid, Team::Radiant)[0].pos;
        run(&mut m, 10.0);
        let after = m.lane_units(Lane::Mid, Team::Radiant)[0].pos;
        assert!(distance(before, after) > 20.0, "{}", distance(before, after));
        run(&mut m, 50.0);
        let dead = m.units.iter().filter(|u| u.lane.is_some() && !u.alive).count();
        let hurt = m.units.iter().filter(|u| u.lane.is_some() && u.alive && u.hp < u.max_hp).count();
        assert!(dead + hurt > 0, "the waves never fought");
    }

    #[test]
    fn every_lane_leader_walks_on_instead_of_freezing_at_a_waypoint() {
        let Some(mut m) = world() else { return };
        run(&mut m, 0.2);
        let spawn: std::collections::HashMap<(Lane, Team), [f32; 3]> = Lane::ALL
            .iter()
            .flat_map(|&l| [Team::Radiant, Team::Dire].map(|t| ((l, t), m.lane_units(l, t)[0].pos)))
            .collect();
        run(&mut m, 25.0);
        for lane in Lane::ALL {
            for team in [Team::Radiant, Team::Dire] {
                let leader = m
                    .lane_units(lane, team)
                    .into_iter()
                    .filter(|u| u.target.is_none())
                    .max_by_key(|u| u.path_index);
                let Some(leader) = leader else { continue };
                assert!(leader.path_index >= 2, "{lane:?} {team:?} stuck at node {}", leader.path_index);
                assert!(distance(leader.pos, spawn[&(lane, team)]) > 120.0, "{lane:?} {team:?} only {}", distance(leader.pos, spawn[&(lane, team)]));
            }
        }
    }

    #[test]
    fn a_tower_shoots_the_creep_in_its_range_and_leaves_the_far_one() {
        let Some(mut m) = world() else { return };
        let tower = m.units.iter().find(|u| u.key == "dota_goodguys_tower1_mid").unwrap().clone();
        let near = m.spawn_unit("npc_dota_creep_badguys_melee", Team::Dire, [tower.pos[0] + 6.0, tower.pos[1], tower.pos[2]]).unwrap();
        let far = m.spawn_unit("npc_dota_creep_badguys_melee", Team::Dire, [tower.pos[0] + 60.0, tower.pos[1], tower.pos[2]]).unwrap();
        let far_hp = m.unit(far).unwrap().hp;
        run(&mut m, 4.0);
        assert!(m.unit(near).map_or(true, |u| u.hp < u.max_hp || !u.alive));
        assert_eq!(m.unit(far).unwrap().hp, far_hp);
    }

    #[test]
    fn a_tower_prefers_creeps_over_a_hero() {
        let Some(mut m) = world() else { return };
        let tower = m.units.iter().find(|u| u.key == "dota_goodguys_tower1_mid").unwrap().clone();
        m.add_hero(2, Team::Dire, "npc_dota_hero_axe", [tower.pos[0] + 5.0, tower.pos[1], tower.pos[2]]).unwrap();
        let creep = m.spawn_unit("npc_dota_creep_badguys_melee", Team::Dire, [tower.pos[0] + 8.0, tower.pos[1], tower.pos[2]]).unwrap();
        assert_eq!(m.nearest_enemy(tower.id).map(|c| c.id), Some(creep));
        m.deal_damage(None, creep, 99999.0, crate::combat::DamageType::Pure);
        assert_eq!(m.nearest_enemy(tower.id).map(|c| c.id), Some(2));
    }

    #[test]
    fn five_minutes_of_lanes_run_without_losing_creeps_or_the_clock() {
        let Some(mut m) = world() else { return };
        run(&mut m, 300.0);
        assert!(m.waves_spawned >= 10);
        assert!(m.units.iter().all(|u| u.pos.iter().all(|c| c.is_finite())));
        let living = m.units.iter().filter(|u| u.alive && u.lane.is_some()).count();
        assert!(living < 400, "{living}");
    }
}
