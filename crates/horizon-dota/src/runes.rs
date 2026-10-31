use crate::combat::{UnitId, distance_xz};
use crate::modifiers::ModKind;
use crate::sim::{DT, Event, Match, PlayerId, TICK_RATE};

pub const RUNE_BASE: UnitId = 900_000;
const PICKUP: f32 = 3.0;
const BOUNTY_EVERY: f32 = 180.0;
const POWER_FIRST: f32 = 360.0;
const POWER_EVERY: f32 = 120.0;
const DOUBLE_DAMAGE_TIME: f32 = 45.0;
const HASTE_TIME: f32 = 22.0;
const HASTE_SPEED: f32 = 550.0;
const REGEN_TIME: f32 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuneKind {
    Bounty,
    DoubleDamage,
    Haste,
    Regeneration,
}

impl RuneKind {
    pub fn key(self) -> &'static str {
        match self {
            RuneKind::Bounty => "rune_bounty",
            RuneKind::DoubleDamage => "rune_dd",
            RuneKind::Haste => "rune_haste",
            RuneKind::Regeneration => "rune_regen",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            RuneKind::Bounty => "Bounty Rune",
            RuneKind::DoubleDamage => "Double Damage Rune",
            RuneKind::Haste => "Haste Rune",
            RuneKind::Regeneration => "Regeneration Rune",
        }
    }

    pub fn sound(self) -> &'static str {
        match self {
            RuneKind::Bounty => "Rune.Bounty",
            RuneKind::DoubleDamage => "Rune.DD",
            RuneKind::Haste => "Rune.Haste",
            RuneKind::Regeneration => "Rune.Regen",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Rune {
    pub id: UnitId,
    pub kind: RuneKind,
    pub pos: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct RuneSpawner {
    pub bounty: bool,
    pub pos: [f32; 3],
}

#[derive(Clone, Debug, Default)]
pub struct RuneState {
    pub spawners: Vec<RuneSpawner>,
    pub runes: Vec<Rune>,
    pub next_bounty: f32,
    pub next_power: f32,
    pub serial: u32,
}

impl Match {
    pub fn setup_runes(&mut self, spawners: Vec<RuneSpawner>) {
        self.rune_state = RuneState {
            spawners,
            runes: Vec::new(),
            next_bounty: 0.0,
            next_power: POWER_FIRST,
            serial: 0,
        };
    }

    fn clock_seconds(&self) -> f32 {
        self.tick as f32 / TICK_RATE as f32
    }

    pub fn bounty_gold(&self) -> i32 {
        50 + (self.clock_seconds() / 60.0 * 3.0) as i32
    }

    fn place_rune(&mut self, kind: RuneKind, pos: [f32; 3]) {
        self.rune_state.serial += 1;
        let id = RUNE_BASE + self.rune_state.serial;
        self.rune_state.runes.push(Rune { id, kind, pos });
        self.emit_sound("General.RunePickUp", pos);
    }

    fn spawn_bounty_runes(&mut self) {
        let spots: Vec<[f32; 3]> = self.rune_state.spawners.iter().filter(|s| s.bounty).map(|s| s.pos).collect();
        for pos in spots {
            let taken = self
                .rune_state
                .runes
                .iter()
                .any(|r| r.kind == RuneKind::Bounty && distance_xz(r.pos, pos) < 1.0);
            if !taken {
                self.place_rune(RuneKind::Bounty, pos);
            }
        }
    }

    fn spawn_power_rune(&mut self) {
        let spots: Vec<[f32; 3]> = self.rune_state.spawners.iter().filter(|s| !s.bounty).map(|s| s.pos).collect();
        if spots.is_empty() {
            return;
        }
        self.rune_state.runes.retain(|r| r.kind == RuneKind::Bounty);
        let spot = spots[(self.roll_between(0.0, spots.len() as f32 - 0.001)) as usize];
        let kinds = [RuneKind::DoubleDamage, RuneKind::Haste, RuneKind::Regeneration];
        let kind = kinds[(self.roll_between(0.0, kinds.len() as f32 - 0.001)) as usize];
        self.place_rune(kind, spot);
    }

    pub fn damage_amp(&self, id: UnitId) -> f32 {
        self.modifiers_of(id)
            .filter_map(|m| match m.kind {
                ModKind::DamageAmp(v) => Some(v),
                _ => None,
            })
            .sum()
    }

    fn apply_rune(&mut self, player: PlayerId, kind: RuneKind) {
        let unit = player as UnitId;
        match kind {
            RuneKind::Bounty => {
                let gold = self.bounty_gold();
                self.grant_gold(player, gold);
            }
            RuneKind::DoubleDamage => {
                self.add_modifier(unit, "modifier_rune_doubledamage", ModKind::DamageAmp(1.0), None, DOUBLE_DAMAGE_TIME);
            }
            RuneKind::Haste => {
                let base = self.hero(player).map_or(300.0, |h| h.stats(&self.rules).move_speed).max(1.0);
                self.add_modifier(unit, "modifier_rune_haste", ModKind::MoveSpeed(HASTE_SPEED / base - 1.0), None, HASTE_TIME);
            }
            RuneKind::Regeneration => {
                self.add_modifier(unit, "modifier_rune_regen", ModKind::Regen(1.0 / REGEN_TIME), None, REGEN_TIME);
            }
        }
        if let Some(pos) = self.hero(player).map(|h| h.pos) {
            self.emit_sound(kind.sound(), pos);
        }
        self.events.push(Event::RunePicked { player, kind });
    }

    pub(crate) fn step_runes(&mut self) {
        if self.rune_state.spawners.is_empty() {
            return;
        }
        self.rune_state.next_bounty -= DT;
        if self.rune_state.next_bounty <= 0.0 {
            self.rune_state.next_bounty = BOUNTY_EVERY;
            self.spawn_bounty_runes();
        }
        self.rune_state.next_power -= DT;
        if self.rune_state.next_power <= 0.0 {
            self.rune_state.next_power = POWER_EVERY;
            self.spawn_power_rune();
        }
        let mut taken: Vec<(UnitId, PlayerId)> = Vec::new();
        for r in &self.rune_state.runes {
            let hero = self
                .heroes
                .iter()
                .filter(|h| h.alive && distance_xz(h.pos, r.pos) <= PICKUP)
                .min_by(|a, b| distance_xz(a.pos, r.pos).total_cmp(&distance_xz(b.pos, r.pos)));
            if let Some(h) = hero {
                taken.push((r.id, h.player));
            }
        }
        for (id, player) in taken {
            if let Some(i) = self.rune_state.runes.iter().position(|r| r.id == id) {
                let rune = self.rune_state.runes.remove(i);
                self.apply_rune(player, rune.kind);
            }
        }
    }

    pub(crate) fn step_regen_modifiers(&mut self) {
        let heals: Vec<(UnitId, f32)> = self
            .modifiers
            .iter()
            .filter_map(|m| match m.kind {
                ModKind::Regen(f) => Some((m.owner, f)),
                _ => None,
            })
            .collect();
        for (owner, fraction) in heals {
            if let Some(h) = self.heroes.iter_mut().find(|h| h.player as UnitId == owner && h.alive) {
                let stats = h.stats(&self.rules);
                h.hp = (h.hp + stats.max_hp * fraction * DT).min(stats.max_hp);
                h.mana = (h.mana + stats.max_mana * fraction * DT).min(stats.max_mana);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Data, dev_data_dir};
    use crate::map::{MapData, find_map_file};
    use crate::sim::{Rules, Team};
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

    fn hero_at(m: &mut Match, pos: [f32; 3]) {
        m.add_hero(1, Team::Radiant, "npc_dota_hero_axe", pos).unwrap();
    }

    #[test]
    fn the_map_has_four_rune_spots_from_the_files() {
        let Some(m) = world() else { return };
        assert_eq!(m.rune_state.spawners.iter().filter(|s| s.bounty).count(), 2);
        assert_eq!(m.rune_state.spawners.iter().filter(|s| !s.bounty).count(), 2);
    }

    #[test]
    fn bounty_runes_appear_at_the_start_and_pay_gold_to_whoever_walks_onto_one() {
        let Some(mut m) = world() else { return };
        let spot = m.rune_state.spawners.iter().find(|s| s.bounty).unwrap().pos;
        hero_at(&mut m, [spot[0] + 30.0, spot[1], spot[2]]);
        run(&mut m, 0.2);
        assert_eq!(m.rune_state.runes.iter().filter(|r| r.kind == RuneKind::Bounty).count(), 2);
        let gold = m.hero(1).unwrap().gold;
        m.heroes[0].pos = spot;
        run(&mut m, 0.2);
        assert!(m.hero(1).unwrap().gold >= gold + 50);
        assert_eq!(m.rune_state.runes.iter().filter(|r| r.kind == RuneKind::Bounty).count(), 1);
    }

    #[test]
    fn a_power_up_rune_comes_at_six_minutes_and_double_damage_doubles_damage() {
        let Some(mut m) = world() else { return };
        hero_at(&mut m, [0.0, 200.0, 0.0]);
        run(&mut m, 359.0);
        assert!(!m.rune_state.runes.iter().any(|r| r.kind != RuneKind::Bounty));
        run(&mut m, 2.0);
        assert_eq!(m.rune_state.runes.iter().filter(|r| r.kind != RuneKind::Bounty).count(), 1);
        m.apply_rune(1, RuneKind::DoubleDamage);
        assert_eq!(m.damage_amp(1), 1.0);
        run(&mut m, 46.0);
        assert_eq!(m.damage_amp(1), 0.0);
    }

    #[test]
    fn haste_sets_speed_to_550_and_regeneration_refills_over_time() {
        let Some(mut m) = world() else { return };
        hero_at(&mut m, [0.0, 200.0, 0.0]);
        let base = m.hero(1).unwrap().stats(&m.rules).move_speed;
        m.apply_rune(1, RuneKind::Haste);
        assert!((base * m.speed_factor(1) - 550.0).abs() < 0.5);
        m.heroes[0].hp = 10.0;
        m.heroes[0].mana = 0.0;
        m.apply_rune(1, RuneKind::Regeneration);
        run(&mut m, 31.0);
        let h = m.hero(1).unwrap();
        let s = h.stats(&m.rules);
        assert!(h.hp >= s.max_hp - 1.0 && h.mana >= s.max_mana - 1.0);
    }
}
