use crate::data::UnitDef;
use crate::sim::{Match, PlayerId, Team, DT};

pub type UnitId = u32;
pub const FIRST_UNIT_ID: UnitId = 1000;
pub const UNIT_METERS: f32 = 0.0254;
pub const ARMOR_FACTOR: f32 = 0.06;
pub const HERO_EYE: f32 = 1.6;
pub const DENY_BELOW: f32 = 0.5;
pub const XP_RANGE: f32 = 1500.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DamageType {
    Physical,
    Magical,
    Pure,
}

pub fn physical_multiplier(armor: f32) -> f32 {
    1.0 - ARMOR_FACTOR * armor / (1.0 + ARMOR_FACTOR * armor.abs())
}

pub fn apply_resistance(amount: f32, kind: DamageType, armor: f32, magic_resist: f32) -> f32 {
    match kind {
        DamageType::Physical => amount * physical_multiplier(armor),
        DamageType::Magical => amount * (1.0 - magic_resist / 100.0),
        DamageType::Pure => amount,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    pub id: UnitId,
    pub team: Team,
    pub name: String,
    pub key: String,
    pub pos: [f32; 3],
    pub hp: f32,
    pub max_hp: f32,
    pub hp_regen: f32,
    pub armor: f32,
    pub magic_resist: f32,
    pub damage: (f32, f32),
    pub attack_rate: f32,
    pub attack_point: f32,
    pub attack_range: f32,
    pub projectile_speed: f32,
    pub melee: bool,
    pub radius: f32,
    pub bounty_gold: (f32, f32),
    pub bounty_xp: f32,
    pub alive: bool,
    pub cooldown: f32,
    pub kind: crate::ai::UnitKind,
    pub ai: bool,
    pub lane: Option<crate::map::Lane>,
    pub path: Vec<[f32; 3]>,
    pub path_index: usize,
    pub move_speed: f32,
    pub acquisition_range: f32,
    pub target: Option<UnitId>,
    pub invulnerable: bool,
    pub dead_for: f32,
    pub neutral: bool,
    pub home: [f32; 3],
    pub aggro: Option<UnitId>,
    pub aggro_time: f32,
    pub sound_set: String,
}

impl Unit {
    pub fn from_def(id: UnitId, team: Team, def: &UnitDef, pos: [f32; 3]) -> Unit {
        Unit {
            id,
            team,
            name: def.name.clone(),
            key: def.name.clone(),
            pos,
            hp: def.health,
            max_hp: def.health,
            hp_regen: def.health_regen,
            armor: def.armor,
            magic_resist: def.magic_resist,
            damage: (def.damage_min, def.damage_max),
            attack_rate: def.attack_rate.max(0.1),
            attack_point: def.attack_point,
            attack_range: def.attack_range,
            projectile_speed: def.projectile_speed,
            melee: def.melee,
            radius: def.raw.f32_or("RingRadius", 45.0),
            bounty_gold: def.bounty_gold,
            bounty_xp: def.bounty_xp,
            alive: true,
            cooldown: 0.0,
            kind: crate::ai::UnitKind::Creep,
            ai: false,
            lane: None,
            path: Vec::new(),
            path_index: 0,
            move_speed: def.move_speed,
            acquisition_range: def.acquisition_range,
            target: None,
            invulnerable: false,
            dead_for: 0.0,
            neutral: false,
            home: pos,
            aggro: None,
            aggro_time: 0.0,
            sound_set: def.raw.str("SoundSet").unwrap_or("").to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Swing {
    pub attacker: UnitId,
    pub target: UnitId,
    pub time_left: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Projectile {
    pub attacker: UnitId,
    pub target: UnitId,
    pub pos: [f32; 3],
    pub speed: f32,
    pub damage: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackReject {
    NoSuchUnit,
    Dead,
    Friendly,
    OutOfRange,
    Cooling,
    Invulnerable,
    Disabled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Combatant {
    pub id: UnitId,
    pub team: Team,
    pub pos: [f32; 3],
    pub radius: f32,
    pub alive: bool,
    pub hp_fraction: f32,
    pub neutral: bool,
}

impl Combatant {
    pub fn hostile_to(&self, other: &Combatant) -> bool {
        self.neutral || other.neutral || self.team != other.team
    }
}

pub fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

pub fn distance_xz(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1]).sqrt()
}

pub fn ray_hits(origin: [f32; 3], dir: [f32; 3], center: [f32; 3], radius: f32) -> Option<f32> {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len < 1e-6 {
        return None;
    }
    let d = [dir[0] / len, dir[1] / len, dir[2] / len];
    let oc = [center[0] - origin[0], center[1] - origin[1], center[2] - origin[2]];
    let t = oc[0] * d[0] + oc[1] * d[1] + oc[2] * d[2];
    if t < 0.0 {
        return None;
    }
    let closest = [origin[0] + d[0] * t, origin[1] + d[1] * t, origin[2] + d[2] * t];
    (distance(closest, center) <= radius).then_some(t)
}

impl Match {
    pub fn spawn_unit(&mut self, name: &str, team: Team, pos: [f32; 3]) -> Option<UnitId> {
        let def = self.data.unit(name)?.clone();
        let id = self.next_unit.max(FIRST_UNIT_ID);
        self.next_unit = id + 1;
        self.units.push(Unit::from_def(id, team, &def, pos));
        Some(id)
    }

    pub fn unit(&self, id: UnitId) -> Option<&Unit> {
        self.units.iter().find(|u| u.id == id)
    }

    pub fn combatant(&self, id: UnitId) -> Option<Combatant> {
        if id >= FIRST_UNIT_ID {
            let u = self.unit(id)?;
            Some(Combatant {
                id,
                team: u.team,
                pos: u.pos,
                radius: u.radius * UNIT_METERS,
                alive: u.alive,
                hp_fraction: u.hp / u.max_hp.max(1.0),
                neutral: u.neutral,
            })
        } else {
            let h = self.hero(id as PlayerId)?;
            let max = h.stats(&self.rules).max_hp;
            Some(Combatant {
                id,
                team: h.team,
                pos: h.pos,
                radius: 0.6,
                alive: h.alive,
                hp_fraction: h.hp / max.max(1.0),
                neutral: false,
            })
        }
    }

    pub fn all_combatants(&self) -> Vec<Combatant> {
        let mut out: Vec<Combatant> = self.heroes.iter().filter_map(|h| self.combatant(h.player)).collect();
        out.extend(self.units.iter().filter_map(|u| self.combatant(u.id)));
        out
    }

    pub fn aimed_at(&self, attacker: UnitId, origin: [f32; 3], dir: [f32; 3], reach: f32) -> Option<UnitId> {
        let me = self.combatant(attacker)?;
        self.all_combatants()
            .into_iter()
            .filter(|c| c.id != attacker && c.alive)
            .filter(|c| c.hostile_to(&me) || c.hp_fraction < DENY_BELOW)
            .filter_map(|c| ray_hits(origin, dir, [c.pos[0], c.pos[1] + 1.0, c.pos[2]], c.radius + 0.4).map(|t| (t - c.radius - 0.4, c.id)))
            .filter(|(t, _)| *t <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
    }

    fn attacker_stats(&self, id: UnitId) -> Option<(f32, f32, f32, f32, f32, f32, bool)> {
        if id >= FIRST_UNIT_ID {
            let u = self.unit(id)?;
            let range = u.attack_range * UNIT_METERS;
            Some((u.damage.0, u.damage.1, u.attack_rate, u.attack_point, range, u.projectile_speed, u.melee))
        } else {
            let h = self.hero(id as PlayerId)?;
            let s = h.stats(&self.rules);
            let interval = h.def.attack_rate / (s.attack_speed / 100.0).max(0.2);
            Some((s.damage_min, s.damage_max, interval, h.def.attack_point, h.def.attack_range * UNIT_METERS, h.def.projectile_speed, h.def.melee))
        }
    }

    pub fn attack(&mut self, attacker: UnitId, target: UnitId) -> Result<(), AttackReject> {
        let me = self.combatant(attacker).ok_or(AttackReject::NoSuchUnit)?;
        let them = self.combatant(target).ok_or(AttackReject::NoSuchUnit)?;
        if !me.alive || !them.alive {
            return Err(AttackReject::Dead);
        }
        if self.stunned(attacker) {
            return Err(AttackReject::Disabled);
        }
        if !me.hostile_to(&them) && them.hp_fraction >= DENY_BELOW {
            return Err(AttackReject::Friendly);
        }
        if target >= FIRST_UNIT_ID && self.unit(target).is_some_and(|u| u.invulnerable) {
            return Err(AttackReject::Invulnerable);
        }
        let (_, _, interval, point, range, ..) = self.attacker_stats(attacker).ok_or(AttackReject::NoSuchUnit)?;
        if distance_xz(me.pos, them.pos) > range + me.radius + them.radius {
            return Err(AttackReject::OutOfRange);
        }
        if self.swings.iter().any(|s| s.attacker == attacker) || self.cooldown_of(attacker) > 0.0 {
            return Err(AttackReject::Cooling);
        }
        self.set_cooldown(attacker, interval);
        self.swings.push(Swing { attacker, target, time_left: point * interval });
        Ok(())
    }

    fn cooldown_of(&self, id: UnitId) -> f32 {
        if id >= FIRST_UNIT_ID {
            self.unit(id).map_or(0.0, |u| u.cooldown)
        } else {
            self.hero(id as PlayerId).map_or(0.0, |h| h.attack_cooldown)
        }
    }

    fn set_cooldown(&mut self, id: UnitId, value: f32) {
        if id >= FIRST_UNIT_ID {
            if let Some(u) = self.units.iter_mut().find(|u| u.id == id) {
                u.cooldown = value;
            }
        } else if let Some(h) = self.heroes.iter_mut().find(|h| h.player == id as PlayerId) {
            h.attack_cooldown = value;
        }
    }

    fn roll(&mut self, low: f32, high: f32) -> f32 {
        self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let unit = ((self.rng >> 33) as f32) / (u32::MAX >> 1) as f32;
        low + (high - low).max(0.0) * unit.clamp(0.0, 1.0)
    }

    pub fn deal_damage(&mut self, attacker: Option<UnitId>, target: UnitId, amount: f32, kind: DamageType) -> f32 {
        if target >= FIRST_UNIT_ID && self.unit(target).is_some_and(|u| u.invulnerable) {
            return 0.0;
        }
        let Some((armor, resist)) = self.defence_of(target) else { return 0.0 };
        let amount = amount * (1.0 + attacker.map_or(0.0, |a| self.damage_amp(a)));
        let dealt = apply_resistance(amount, kind, armor, resist).max(0.0);
        let killed = if target >= FIRST_UNIT_ID {
            match self.units.iter_mut().find(|u| u.id == target && u.alive) {
                Some(u) => {
                    u.hp -= dealt;
                    if u.neutral && attacker.is_some() {
                        u.aggro = attacker;
                        u.aggro_time = 8.0;
                    }
                    if u.hp <= 0.0 {
                        u.hp = 0.0;
                        u.alive = false;
                        true
                    } else {
                        false
                    }
                }
                None => false,
            }
        } else {
            let before = self.hero(target as PlayerId).is_some_and(|h| h.alive);
            self.damage_hero_raw(target as PlayerId, dealt);
            before && self.hero(target as PlayerId).is_some_and(|h| !h.alive)
        };
        if killed {
            self.on_kill(attacker, target);
        }
        dealt
    }

    pub fn sound_set_of(&self, id: UnitId) -> String {
        if id >= FIRST_UNIT_ID {
            self.unit(id).map(|u| u.sound_set.clone()).unwrap_or_default()
        } else {
            self.hero(id as PlayerId)
                .map(|h| h.def.raw.str("SoundSet").unwrap_or("").to_string())
                .unwrap_or_default()
        }
    }

    fn defence_of(&self, id: UnitId) -> Option<(f32, f32)> {
        if id >= FIRST_UNIT_ID {
            self.unit(id).filter(|u| u.alive).map(|u| (u.armor + self.armor_bonus(id), u.magic_resist))
        } else {
            let h = self.hero(id as PlayerId).filter(|h| h.alive)?;
            let s = h.stats(&self.rules);
            Some((s.armor + self.armor_bonus(id), s.magic_resist))
        }
    }

    fn on_kill(&mut self, killer: Option<UnitId>, victim: UnitId) {
        let Some(v) = self.combatant(victim) else { return };
        if victim >= FIRST_UNIT_ID && self.unit(victim).is_some_and(|u| u.kind.is_building()) {
            self.refresh_invulnerability();
            if self.winner.is_none() && self.unit(victim).is_some_and(|u| u.kind == crate::ai::UnitKind::Fort) {
                let winner = v.team.enemy();
                self.winner = Some(winner);
                self.events.push(crate::sim::Event::GameOver { winner });
            }
        }
        let (gold, xp) = if victim >= FIRST_UNIT_ID {
            let u = self.unit(victim).cloned();
            match u {
                Some(u) => {
                    let g = self.roll(u.bounty_gold.0, u.bounty_gold.1).round() as i32;
                    (g, u.bounty_xp as i32)
                }
                None => (0, 0),
            }
        } else {
            (0, 0)
        };
        let killer_combatant = killer.and_then(|k| self.combatant(k));
        let denied = !v.neutral && killer_combatant.as_ref().is_some_and(|k| k.team == v.team);
        let roshan = victim >= FIRST_UNIT_ID && self.unit(victim).is_some_and(|u| u.kind == crate::ai::UnitKind::Neutral);
        let death_sound = if roshan {
            "Roshan.Death".to_string()
        } else if victim >= FIRST_UNIT_ID && self.unit(victim).is_some_and(|u| u.kind == crate::ai::UnitKind::Tower) {
            if v.team == crate::sim::Team::Radiant { "Building_RadiantTower.Destruction".into() } else { "Building_DireTower.Destruction".into() }
        } else {
            format!("{}.Death", self.sound_set_of(victim))
        };
        self.emit_sound(&death_sound, v.pos);
        if roshan {
            self.on_roshan_killed(killer, victim);
        }
        self.events.push(crate::sim::Event::Killed { killer, victim, denied });
        if let Some(k) = killer {
            if k < FIRST_UNIT_ID {
                if denied {
                    if let Some(h) = self.heroes.iter_mut().find(|h| h.player == k as PlayerId) {
                        h.denies += 1;
                    }
                } else {
                    if !roshan {
                        self.grant_gold(k as PlayerId, gold);
                    }
                    if victim >= FIRST_UNIT_ID {
                        if let Some(h) = self.heroes.iter_mut().find(|h| h.player == k as PlayerId) {
                            h.last_hits += 1;
                        }
                    } else if let Some(h) = self.heroes.iter_mut().find(|h| h.player == k as PlayerId) {
                        h.kills += 1;
                    }
                }
            }
        }
        if !denied && xp > 0 {
            let enemy = if v.neutral {
                killer_combatant.as_ref().map_or(v.team.enemy(), |k| k.team)
            } else {
                v.team.enemy()
            };
            let range = XP_RANGE * UNIT_METERS;
            let receivers: Vec<PlayerId> = self
                .heroes
                .iter()
                .filter(|h| h.alive && h.team == enemy && distance_xz(h.pos, v.pos) <= range)
                .map(|h| h.player)
                .collect();
            if !receivers.is_empty() {
                let share = xp / receivers.len() as i32;
                for p in receivers {
                    self.grant_xp(p, share);
                }
            }
        }
    }

    pub(crate) fn step_combat(&mut self) {
        for u in &mut self.units {
            if u.alive {
                u.hp = (u.hp + u.hp_regen * DT).min(u.max_hp);
                u.cooldown = (u.cooldown - DT).max(0.0);
                if u.aggro_time > 0.0 {
                    u.aggro_time -= DT;
                    if u.aggro_time <= 0.0 {
                        u.aggro = None;
                    }
                }
            }
        }
        for h in &mut self.heroes {
            h.attack_cooldown = (h.attack_cooldown - DT).max(0.0);
        }
        let mut finished = Vec::new();
        for swing in &mut self.swings {
            swing.time_left -= DT;
            if swing.time_left <= 0.0 {
                finished.push(swing.clone());
            }
        }
        self.swings.retain(|s| s.time_left > 0.0);
        for swing in finished {
            let alive = self.combatant(swing.attacker).is_some_and(|c| c.alive);
            if !alive {
                continue;
            }
            let Some((low, high, _, _, range, speed, melee)) = self.attacker_stats(swing.attacker) else { continue };
            let amount = self.roll(low, high);
            let (Some(me), Some(them)) = (self.combatant(swing.attacker), self.combatant(swing.target)) else { continue };
            let set = self.sound_set_of(swing.attacker);
            self.emit_sound(&format!("{set}.Attack"), me.pos);
            if !them.alive {
                continue;
            }
            if melee || speed <= 0.0 {
                if distance_xz(me.pos, them.pos) <= range + me.radius + them.radius + 1.0 {
                    self.deal_damage(Some(swing.attacker), swing.target, amount, DamageType::Physical);
                    self.on_attacked(swing.target, swing.attacker);
                }
            } else {
                self.projectiles.push(Projectile {
                    attacker: swing.attacker,
                    target: swing.target,
                    pos: me.pos,
                    speed: speed * UNIT_METERS,
                    damage: amount,
                });
            }
        }
        let mut arrived = Vec::new();
        for p in &mut self.projectiles {
            let Some(target) = self.heroes.iter().find(|h| h.player == p.target as PlayerId && p.target < FIRST_UNIT_ID).map(|h| h.pos).or_else(|| self.units.iter().find(|u| u.id == p.target).map(|u| u.pos)) else {
                p.speed = -1.0;
                continue;
            };
            let d = distance(p.pos, target);
            let step = p.speed * DT;
            if d <= step + 0.3 {
                arrived.push(p.clone());
                p.speed = -1.0;
            } else {
                for i in 0..3 {
                    p.pos[i] += (target[i] - p.pos[i]) / d * step;
                }
            }
        }
        self.projectiles.retain(|p| p.speed > 0.0);
        for p in arrived {
            if self.combatant(p.target).is_some_and(|c| c.alive) {
                self.deal_damage(Some(p.attacker), p.target, p.damage, DamageType::Physical);
                self.on_attacked(p.target, p.attacker);
            }
        }
        for u in &mut self.units {
            if !u.alive {
                u.dead_for += DT;
            }
        }
        self.units.retain(|u| u.alive || u.kind.is_building() || u.dead_for < 3.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Data, dev_data_dir};
    use crate::sim::{Event, Rules, TICK_RATE};
    use std::sync::Arc;

    fn arena() -> Option<Match> {
        let data = Arc::new(Data::load(&dev_data_dir()?).ok()?);
        let mut m = Match::new(data, Rules::default());
        m.add_hero(1, Team::Radiant, "npc_dota_hero_axe", [0.0, 0.0, 0.0]).ok()?;
        Some(m)
    }

    fn run(m: &mut Match, seconds: f32) {
        for _ in 0..(seconds * TICK_RATE as f32) as u32 {
            m.step();
        }
    }

    #[test]
    fn the_armor_curve_is_the_dota_one() {
        assert!((physical_multiplier(0.0) - 1.0).abs() < 1e-6);
        assert!((physical_multiplier(10.0) - (1.0 - 0.6 / 1.6)).abs() < 1e-6);
        assert!(physical_multiplier(-10.0) > 1.0);
        assert!(physical_multiplier(1000.0) > 0.0);
        assert!((apply_resistance(100.0, DamageType::Magical, 50.0, 25.0) - 75.0).abs() < 1e-4);
        assert_eq!(apply_resistance(100.0, DamageType::Pure, 50.0, 25.0), 100.0);
    }

    #[test]
    fn a_ray_picks_the_nearest_unit_it_passes_through() {
        let o = [0.0, 1.6, 0.0];
        let d = [0.0, 0.0, 1.0];
        assert_eq!(ray_hits(o, d, [0.0, 1.6, 5.0], 0.5), Some(5.0));
        assert_eq!(ray_hits(o, d, [2.0, 1.6, 5.0], 0.5), None);
        assert_eq!(ray_hits(o, d, [0.0, 1.6, -5.0], 0.5), None);
        assert_eq!(ray_hits(o, [0.0; 3], [0.0, 1.6, 5.0], 0.5), None);
    }

    #[test]
    fn a_melee_hero_kills_a_creep_and_collects_the_bounty_from_the_files() {
        let Some(mut m) = arena() else { return };
        let creep = m.spawn_unit("npc_dota_creep_badguys_melee", Team::Dire, [1.0, 0.0, 0.0]).unwrap();
        let def = m.data.unit("npc_dota_creep_badguys_melee").unwrap().clone();
        let gold_before = m.hero(1).unwrap().gold;
        let mut hits = 0;
        for _ in 0..1500 {
            if m.attack(1, creep).is_ok() {
                hits += 1;
            }
            m.step();
            if !m.unit(creep).map_or(false, |u| u.alive) {
                break;
            }
        }
        assert!(!m.unit(creep).is_some_and(|u| u.alive));
        assert!(hits >= 3 && hits <= 16, "{hits}");
        let hero = m.hero(1).unwrap();
        assert_eq!(hero.last_hits, 1);
        let gained = hero.gold - gold_before;
        let passive = (m.time() / m.rules.passive_gold_interval) as i32 * m.rules.passive_gold_amount;
        assert!(gained - passive >= def.bounty_gold.0 as i32 - 1 && gained - passive <= def.bounty_gold.1 as i32 + 1, "{gained} {passive}");
        assert!(hero.xp > 0);
    }

    #[test]
    fn attacks_need_range_and_wait_for_the_cooldown() {
        let Some(mut m) = arena() else { return };
        let far = m.spawn_unit("npc_dota_creep_badguys_melee", Team::Dire, [30.0, 0.0, 0.0]).unwrap();
        assert_eq!(m.attack(1, far), Err(AttackReject::OutOfRange));
        let near = m.spawn_unit("npc_dota_creep_badguys_melee", Team::Dire, [1.0, 0.0, 0.0]).unwrap();
        assert_eq!(m.attack(1, near), Ok(()));
        assert_eq!(m.attack(1, near), Err(AttackReject::Cooling));
        let interval = m.hero(1).unwrap().def.attack_rate / (m.hero(1).unwrap().stats(&m.rules).attack_speed / 100.0);
        run(&mut m, interval + 0.1);
        assert_eq!(m.attack(1, near), Ok(()));
    }

    #[test]
    fn damage_is_cut_by_armor() {
        let Some(mut m) = arena() else { return };
        let creep = m.spawn_unit("npc_dota_creep_badguys_melee", Team::Dire, [1.0, 0.0, 0.0]).unwrap();
        let armor = m.unit(creep).unwrap().armor;
        let before = m.unit(creep).unwrap().hp;
        let dealt = m.deal_damage(Some(1), creep, 100.0, DamageType::Physical);
        assert!((dealt - 100.0 * physical_multiplier(armor)).abs() < 1e-3);
        assert!((m.unit(creep).unwrap().hp - (before - dealt)).abs() < 1e-3);
    }

    #[test]
    fn allies_can_only_be_denied_when_hurt() {
        let Some(mut m) = arena() else { return };
        let ally = m.spawn_unit("npc_dota_creep_goodguys_melee", Team::Radiant, [1.0, 0.0, 0.0]).unwrap();
        assert_eq!(m.attack(1, ally), Err(AttackReject::Friendly));
        let max = m.unit(ally).unwrap().max_hp;
        m.units.iter_mut().find(|u| u.id == ally).unwrap().hp = max * 0.3;
        assert_eq!(m.attack(1, ally), Ok(()));
        for _ in 0..400 {
            m.attack(1, ally).ok();
            m.step();
        }
        assert!(!m.unit(ally).is_some_and(|u| u.alive));
        assert_eq!(m.hero(1).unwrap().denies, 1);
        assert_eq!(m.hero(1).unwrap().gold - 600, (m.time() / m.rules.passive_gold_interval) as i32 * m.rules.passive_gold_amount);
        assert!(m.drain_events().iter().any(|e| matches!(e, Event::Killed { denied: true, .. })));
    }

    #[test]
    fn a_ranged_attack_flies_before_it_lands() {
        let Some(mut m) = arena() else { return };
        m.add_hero(2, Team::Dire, "npc_dota_hero_drow_ranger", [0.0, 0.0, 8.0]).unwrap();
        let hp = m.hero(1).unwrap().hp;
        assert_eq!(m.attack(2, 1), Ok(()));
        run(&mut m, 0.05);
        assert_eq!(m.hero(1).unwrap().hp, hp);
        assert_eq!(m.projectiles.len(), 0);
        run(&mut m, 1.0);
        assert!(m.hero(1).unwrap().hp < hp);
    }

    #[test]
    fn killing_a_hero_counts_the_kill() {
        let Some(mut m) = arena() else { return };
        m.add_hero(2, Team::Dire, "npc_dota_hero_pudge", [1.0, 0.0, 0.0]).unwrap();
        m.heroes[1].hp = 1.0;
        assert_eq!(m.attack(1, 2), Ok(()));
        run(&mut m, 1.0);
        assert!(!m.hero(2).unwrap().alive);
        assert_eq!(m.hero(1).unwrap().kills, 1);
        assert_eq!(m.hero(2).unwrap().deaths, 1);
    }

    #[test]
    fn a_big_target_is_reachable_by_its_edge_not_its_centre() {
        let Some(mut m) = arena() else { return };
        let tower = m.spawn_unit("npc_dota_badguys_tower1_top", Team::Dire, [0.0, 0.0, 7.0]).unwrap();
        let radius = m.unit(tower).unwrap().radius * UNIT_METERS;
        assert!(radius > 2.0, "{radius}");
        let origin = [0.0, 0.0, 0.0];
        assert_eq!(m.aimed_at(1, origin, [0.0, 0.0, 1.0], 5.3), Some(tower));
        let far = m.spawn_unit("npc_dota_badguys_tower1_top", Team::Dire, [0.0, 0.0, 30.0]).unwrap();
        m.units.retain(|u| u.id != tower);
        assert_eq!(m.aimed_at(1, origin, [0.0, 0.0, 1.0], 5.3), None);
        let _ = far;
    }

    #[test]
    fn the_aim_picks_enemies_and_ignores_healthy_allies() {
        let Some(mut m) = arena() else { return };
        let ally = m.spawn_unit("npc_dota_creep_goodguys_melee", Team::Radiant, [0.0, 1.0, 2.0]).unwrap();
        let enemy = m.spawn_unit("npc_dota_creep_badguys_melee", Team::Dire, [0.0, 1.0, 3.0]).unwrap();
        let origin = [0.0, 1.0, 0.0];
        assert_eq!(m.aimed_at(1, origin, [0.0, 0.0, 1.0], 10.0), Some(enemy));
        m.units.iter_mut().find(|u| u.id == ally).unwrap().hp = 5.0;
        assert_eq!(m.aimed_at(1, origin, [0.0, 0.0, 1.0], 10.0), Some(ally));
        assert_eq!(m.aimed_at(1, origin, [1.0, 0.0, 0.0], 10.0), None);
    }
}
