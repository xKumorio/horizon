use crate::combat::{DamageType, FIRST_UNIT_ID, HERO_EYE, UNIT_METERS, UnitId, distance_xz};
use crate::data::AbilityDef;
use crate::modifiers::ModKind;
use crate::sim::{Aim, DT, Match};

#[derive(Clone, Debug, PartialEq)]
pub struct PendingCast {
    pub caster: UnitId,
    pub name: String,
    pub level: usize,
    pub aim: Aim,
    pub delay: f32,
}

pub fn damage_kind(def: &AbilityDef) -> DamageType {
    if def.damage_type.contains("MAGICAL") {
        DamageType::Magical
    } else if def.damage_type.contains("PHYSICAL") || def.damage_type.contains("ArmorPhysical") {
        DamageType::Physical
    } else {
        DamageType::Pure
    }
}

impl Match {
    pub fn eye_of(&self, id: UnitId) -> Option<[f32; 3]> {
        let c = self.combatant(id)?;
        Some([c.pos[0], c.pos[1] + HERO_EYE, c.pos[2]])
    }

    pub fn resolve_unit_target(&self, caster: UnitId, aim: Aim, range: f32) -> Option<UnitId> {
        match aim {
            Aim::Unit(id) => {
                let me = self.combatant(caster)?;
                let t = self.combatant(id)?;
                (t.alive && t.hostile_to(&me) && distance_xz(me.pos, t.pos) <= range + t.radius).then_some(id)
            }
            Aim::Direction(dir) => {
                let origin = self.eye_of(caster)?;
                self.aimed_at(caster, origin, dir, range + 1.5)
            }
            _ => None,
        }
    }

    pub fn enemies_in_radius(&self, caster: UnitId, center: [f32; 3], radius: f32) -> Vec<UnitId> {
        let Some(me) = self.combatant(caster) else { return Vec::new() };
        self.all_combatants()
            .into_iter()
            .filter(|c| c.alive && c.hostile_to(&me))
            .filter(|c| !(c.id >= FIRST_UNIT_ID && self.unit(c.id).is_some_and(|u| u.invulnerable)))
            .filter(|c| distance_xz(center, c.pos) <= radius + c.radius)
            .map(|c| c.id)
            .collect()
    }

    pub fn allies_in_radius(&self, caster: UnitId, center: [f32; 3], radius: f32) -> Vec<UnitId> {
        let Some(me) = self.combatant(caster) else { return Vec::new() };
        self.all_combatants()
            .into_iter()
            .filter(|c| c.alive && c.team == me.team && !c.neutral)
            .filter(|c| distance_xz(center, c.pos) <= radius + c.radius)
            .map(|c| c.id)
            .collect()
    }

    pub(crate) fn begin_cast(&mut self, caster: UnitId, name: &str, level: usize, aim: Aim) {
        let delay = self.data.ability(name).map_or(0.0, |d| d.cast_point);
        self.pending_casts.push(PendingCast {
            caster,
            name: name.to_string(),
            level,
            aim,
            delay,
        });
    }

    pub(crate) fn step_casts(&mut self) {
        let mut due = Vec::new();
        for c in &mut self.pending_casts {
            c.delay -= DT;
            if c.delay <= 0.0 {
                due.push(c.clone());
            }
        }
        self.pending_casts.retain(|c| c.delay > 0.0);
        for c in due {
            let alive = self.combatant(c.caster).is_some_and(|x| x.alive);
            if !alive || self.stunned(c.caster) {
                continue;
            }
            self.execute(c);
        }
    }

    pub(crate) fn on_attacked(&mut self, target: UnitId, _attacker: UnitId) {
        if target >= FIRST_UNIT_ID {
            return;
        }
        let Some(index) = self.heroes.iter().position(|h| h.player == target) else { return };
        let Some(slot) = self.heroes[index].def.abilities.iter().position(|a| a == "axe_counter_helix") else { return };
        let level = self.heroes[index].ability_levels[slot] as usize;
        if level == 0 {
            return;
        }
        let data = self.data.clone();
        let Some(def) = data.ability("axe_counter_helix") else { return };
        let trigger = def.value("trigger_attacks", level).max(1.0) as u32;
        self.heroes[index].counters[slot] += 1;
        if self.heroes[index].counters[slot] < trigger {
            return;
        }
        self.heroes[index].counters[slot] = 0;
        let radius = def.value("radius", level) * UNIT_METERS;
        let damage = def.value("damage", level);
        let center = self.heroes[index].pos;
        for id in self.enemies_in_radius(target, center, radius) {
            self.deal_damage(Some(target), id, damage, damage_kind(def));
        }
        self.events.push(crate::sim::Event::Cast {
            player: target,
            ability: "axe_counter_helix".into(),
            level: level as u32,
            aim: Aim::None,
        });
    }

    fn execute(&mut self, c: PendingCast) {
        let data = self.data.clone();
        let Some(def) = data.ability(&c.name) else { return };
        let v = |key: &str| def.value(key, c.level);
        let range = def.range_at(c.level) * UNIT_METERS;
        let kind = damage_kind(def);
        let Some(me) = self.combatant(c.caster) else { return };
        if let Some(sound) = def.raw.str("AbilitySound") {
            self.emit_sound(sound, me.pos);
        }
        match c.name.as_str() {
            "axe_berserkers_call" => {
                let duration = v("duration");
                for id in self.enemies_in_radius(c.caster, me.pos, v("radius") * UNIT_METERS) {
                    self.add_modifier(id, "modifier_axe_berserkers_call", ModKind::Taunt(c.caster), Some(c.caster), duration);
                }
                self.add_modifier(c.caster, "modifier_axe_berserkers_call_armor", ModKind::Armor(v("bonus_armor")), Some(c.caster), duration);
            }
            "axe_battle_hunger" => {
                let Some(t) = self.resolve_unit_target(c.caster, c.aim, range) else { return };
                let duration = v("duration");
                self.add_modifier(t, "modifier_axe_battle_hunger", ModKind::Dot { per_second: v("damage_per_second"), kind }, Some(c.caster), duration);
                self.add_modifier(t, "modifier_axe_battle_hunger_slow", ModKind::Slow(v("slow") / 100.0), Some(c.caster), duration);
            }
            "axe_culling_blade" => {
                let Some(t) = self.resolve_unit_target(c.caster, c.aim, range) else { return };
                let damage = v("damage");
                let hp = self.combatant(t).map_or(0.0, |x| x.hp_fraction);
                let hp_now = self.hp_of(t);
                let _ = hp;
                if hp_now <= damage {
                    self.deal_damage(Some(c.caster), t, hp_now + 1.0, DamageType::Pure);
                    let speed = v("speed_bonus") / 100.0;
                    let duration = v("speed_duration");
                    for ally in self.allies_in_radius(c.caster, me.pos, v("speed_aoe") * UNIT_METERS) {
                        self.add_modifier(ally, "modifier_axe_culling_blade_speed", ModKind::MoveSpeed(speed), Some(c.caster), duration);
                    }
                } else {
                    self.deal_damage(Some(c.caster), t, damage, DamageType::Pure);
                }
            }
            _ => self.generic_effect(&c, def, me.pos, range, kind),
        }
    }

    pub fn hp_of(&self, id: UnitId) -> f32 {
        if id >= FIRST_UNIT_ID {
            self.unit(id).map_or(0.0, |u| u.hp)
        } else {
            self.hero(id).map_or(0.0, |h| h.hp)
        }
    }

    fn generic_effect(&mut self, c: &PendingCast, def: &AbilityDef, origin: [f32; 3], range: f32, kind: DamageType) {
        let level = c.level;
        let value = |k: &str| def.value(k, level);
        let damage = {
            let d = value("damage");
            if d > 0.0 { d } else { crate::data::at(&def.damage, level) }
        };
        let radius = value("radius") * UNIT_METERS;
        let stun = value("stun_duration").max(value("stun"));
        let mut hit = Vec::new();
        if def.has_behavior("NO_TARGET") && radius > 0.0 {
            hit = self.enemies_in_radius(c.caster, origin, radius);
        } else if def.has_behavior("UNIT_TARGET") {
            hit.extend(self.resolve_unit_target(c.caster, c.aim, range));
        } else if def.has_behavior("POINT") && radius > 0.0 {
            let centre = match c.aim {
                Aim::Point(p) => p,
                Aim::Direction(d) => {
                    let len = (d[0] * d[0] + d[2] * d[2]).sqrt().max(1e-3);
                    [origin[0] + d[0] / len * range.min(radius * 4.0 + 6.0), origin[1], origin[2] + d[2] / len * range.min(radius * 4.0 + 6.0)]
                }
                _ => origin,
            };
            hit = self.enemies_in_radius(c.caster, centre, radius);
        }
        for id in hit {
            if damage > 0.0 {
                self.deal_damage(Some(c.caster), id, damage, kind);
            }
            if stun > 0.0 {
                self.add_modifier(id, &format!("modifier_{}_stun", c.name), ModKind::Stun, Some(c.caster), stun);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Data, dev_data_dir};
    use crate::sim::{Intent, Reject, Rules, TICK_RATE, Team};
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

    fn slot_of(m: &Match, name: &str) -> usize {
        m.hero(1).unwrap().def.abilities.iter().position(|a| a == name).unwrap()
    }

    fn learn(m: &mut Match, level_to: u32, name: &str) {
        m.grant_xp(1, 100000);
        let slot = slot_of(m, name);
        for _ in 0..level_to {
            m.submit(1, Intent::LevelUp(slot)).unwrap();
        }
    }

    fn spawn_enemy(m: &mut Match, at: [f32; 3]) -> UnitId {
        m.spawn_unit("npc_dota_creep_badguys_melee", Team::Dire, at).unwrap()
    }

    #[test]
    fn berserkers_call_taunts_what_is_in_its_radius_and_arms_axe() {
        let Some(mut m) = arena() else { return };
        learn(&mut m, 1, "axe_berserkers_call");
        let near = spawn_enemy(&mut m, [4.0, 0.0, 0.0]);
        let far = spawn_enemy(&mut m, [20.0, 0.0, 0.0]);
        let slot = slot_of(&m, "axe_berserkers_call");
        assert_eq!(m.submit(1, Intent::Cast { slot, aim: Aim::None }), Ok(()));
        assert!(m.taunted_by(near).is_none(), "the cast point has not passed yet");
        run(&mut m, 0.5);
        assert_eq!(m.taunted_by(near), Some(1));
        assert_eq!(m.taunted_by(far), None);
        assert_eq!(m.armor_bonus(1), 12.0);
        run(&mut m, 3.0);
        assert_eq!(m.taunted_by(near), None);
        assert_eq!(m.armor_bonus(1), 0.0);
    }

    #[test]
    fn a_cast_and_a_blow_make_the_sounds_the_data_names() {
        let Some(mut m) = arena() else { return };
        learn(&mut m, 1, "axe_berserkers_call");
        let creep = spawn_enemy(&mut m, [1.5, 0.0, 0.0]);
        let slot = slot_of(&m, "axe_berserkers_call");
        m.submit(1, Intent::Cast { slot, aim: Aim::None }).unwrap();
        m.attack(1, creep).unwrap();
        run(&mut m, 1.0);
        let heard: Vec<String> = m
            .drain_events()
            .into_iter()
            .filter_map(|e| match e {
                crate::sim::Event::Sound { event, .. } => Some(event),
                _ => None,
            })
            .collect();
        assert!(heard.contains(&"Hero_Axe.Berserkers_Call".to_string()), "{heard:?}");
        assert!(heard.contains(&"Hero_Axe.Attack".to_string()), "{heard:?}");
    }

    #[test]
    fn what_a_match_asks_the_speakers_for_is_mostly_installed() {
        let Some(root) = crate::data::find_data_dir() else { return };
        let Ok(text) = std::fs::read_to_string(root.join("sound_manifest.json")) else { return };
        let installed: Vec<String> = serde_json::from_str(&text).unwrap();
        let Some(mut m) = arena() else { return };
        m.add_hero(2, Team::Dire, "npc_dota_hero_pudge", [1.0, 0.0, 0.0]).unwrap();
        learn(&mut m, 1, "axe_berserkers_call");
        let creep = spawn_enemy(&mut m, [1.5, 0.0, 0.0]);
        for _ in 0..200 {
            let _ = m.attack(1, creep);
            let _ = m.attack(2, 1);
            m.step();
        }
        let mut asked: Vec<String> = m
            .drain_events()
            .into_iter()
            .filter_map(|e| match e {
                crate::sim::Event::Sound { event, .. } => Some(event),
                _ => None,
            })
            .collect();
        asked.sort();
        asked.dedup();
        assert!(!asked.is_empty());
        let missing: Vec<&String> = asked
            .iter()
            .filter(|e| e.ends_with(".Attack") || e.starts_with("Hero_"))
            .filter(|e| !installed.contains(&e.to_ascii_lowercase()))
            .collect();
        assert!(missing.is_empty(), "attack and hero sounds that were asked for but are not installed: {missing:?}");
    }

    #[test]
    fn a_taunted_creep_goes_for_axe_even_with_another_enemy_closer() {
        let Some(mut m) = arena() else { return };
        let creep = spawn_enemy(&mut m, [5.0, 0.0, 0.0]);
        m.add_hero(2, Team::Radiant, "npc_dota_hero_sven", [4.0, 0.0, 0.0]).unwrap();
        m.units.iter_mut().find(|u| u.id == creep).unwrap().ai = true;
        m.add_modifier(creep, "taunt", ModKind::Taunt(1), Some(1), 3.0);
        run(&mut m, 0.2);
        assert_eq!(m.unit(creep).unwrap().target, Some(1));
    }

    #[test]
    fn battle_hunger_burns_and_slows_for_its_whole_duration() {
        let Some(mut m) = arena() else { return };
        learn(&mut m, 1, "axe_battle_hunger");
        let target = spawn_enemy(&mut m, [0.0, 0.0, 6.0]);
        let slot = slot_of(&m, "axe_battle_hunger");
        let aim = Aim::Direction([0.0, 0.0, 1.0]);
        m.heroes[0].pos = [0.0, 0.0, 0.0];
        assert_eq!(m.submit(1, Intent::Cast { slot, aim }), Ok(()));
        run(&mut m, 0.5);
        assert!(m.modifiers_of(target).any(|x| x.name == "modifier_axe_battle_hunger"));
        let slow = m.speed_factor(target);
        assert!((slow - 0.82).abs() < 1e-4, "{slow}");
        let hp = m.unit(target).unwrap().hp;
        run(&mut m, 5.0);
        let lost = hp - m.unit(target).unwrap().hp;
        assert!(lost >= 48.0 && lost <= 72.0, "{lost}");
        run(&mut m, 10.0);
        assert_eq!(m.modifiers_of(target).count(), 0);
    }

    #[test]
    fn a_unit_target_spell_needs_a_target_and_spends_nothing_without_one() {
        let Some(mut m) = arena() else { return };
        learn(&mut m, 1, "axe_battle_hunger");
        let slot = slot_of(&m, "axe_battle_hunger");
        let mana = m.hero(1).unwrap().mana;
        let aim = Aim::Direction([0.0, 0.0, 1.0]);
        assert_eq!(m.submit(1, Intent::Cast { slot, aim }), Err(Reject::NoTarget));
        assert_eq!(m.hero(1).unwrap().mana, mana);
        assert_eq!(m.hero(1).unwrap().cooldowns[slot], 0.0);
    }

    #[test]
    fn culling_blade_executes_the_weak_and_only_wounds_the_healthy() {
        let Some(mut m) = arena() else { return };
        learn(&mut m, 1, "axe_culling_blade");
        let slot = slot_of(&m, "axe_culling_blade");
        let weak = spawn_enemy(&mut m, [0.0, 0.0, 3.0]);
        m.units.iter_mut().find(|u| u.id == weak).unwrap().hp = 100.0;
        let aim = Aim::Direction([0.0, 0.0, 1.0]);
        assert_eq!(m.submit(1, Intent::Cast { slot, aim }), Ok(()));
        run(&mut m, 0.5);
        assert!(!m.unit(weak).is_some_and(|u| u.alive));
        m.heroes[0].cooldowns[slot] = 0.0;
        m.heroes[0].mana = 500.0;
        let strong = spawn_enemy(&mut m, [0.0, 0.0, 3.0]);
        let before = m.unit(strong).unwrap().hp;
        assert_eq!(m.submit(1, Intent::Cast { slot, aim }), Ok(()));
        run(&mut m, 0.5);
        let after = m.unit(strong).unwrap().hp;
        assert!((before - after - 275.0).abs() < 1.0, "{before} {after}");
    }

    #[test]
    fn counter_helix_spins_on_the_nth_attack_taken() {
        let Some(mut m) = arena() else { return };
        learn(&mut m, 1, "axe_counter_helix");
        let enemy = spawn_enemy(&mut m, [2.0, 0.0, 0.0]);
        let hp = m.unit(enemy).unwrap().hp;
        let trigger = m.data.ability("axe_counter_helix").unwrap().value("trigger_attacks", 1) as u32;
        for n in 1..=trigger {
            m.on_attacked(1, enemy);
            let now = m.unit(enemy).unwrap().hp;
            if n < trigger {
                assert_eq!(now, hp, "attack {n}");
            }
        }
        let helix = m.data.ability("axe_counter_helix").unwrap().value("damage", 1);
        let lost = hp - m.unit(enemy).unwrap().hp;
        assert!((lost - helix).abs() < 1.0, "{lost} {helix}");
        let slot = slot_of(&m, "axe_counter_helix");
        assert_eq!(m.hero(1).unwrap().counters[slot], 0);
    }

    #[test]
    fn a_stunned_hero_cannot_attack_cast_or_finish_a_cast_in_progress() {
        let Some(mut m) = arena() else { return };
        learn(&mut m, 1, "axe_berserkers_call");
        let enemy = spawn_enemy(&mut m, [1.0, 0.0, 0.0]);
        let slot = slot_of(&m, "axe_berserkers_call");
        assert_eq!(m.submit(1, Intent::Cast { slot, aim: Aim::None }), Ok(()));
        m.add_modifier(1, "stun", ModKind::Stun, None, 2.0);
        run(&mut m, 0.5);
        assert_eq!(m.taunted_by(enemy), None, "a stunned caster loses the cast");
        assert_eq!(m.attack(1, enemy), Err(crate::combat::AttackReject::Disabled));
        m.heroes[0].cooldowns[slot] = 0.0;
        assert_eq!(m.submit(1, Intent::Cast { slot, aim: Aim::None }), Err(Reject::Disabled));
    }

    #[test]
    fn any_other_hero_with_a_damage_and_radius_spell_hurts_what_stands_in_it() {
        let Some(data) = dev_data_dir().and_then(|d| Data::load(&d).ok()) else { return };
        let mut m = Match::new(Arc::new(data), Rules::default());
        m.add_hero(1, Team::Radiant, "npc_dota_hero_sven", [0.0; 3]).unwrap();
        m.grant_xp(1, 100000);
        let slot = m.hero(1).unwrap().def.abilities.iter().position(|a| a == "sven_storm_bolt").unwrap();
        m.submit(1, Intent::LevelUp(slot)).unwrap();
        let target = spawn_enemy(&mut m, [0.0, 0.0, 8.0]);
        let hp = m.unit(target).unwrap().hp;
        assert_eq!(m.submit(1, Intent::Cast { slot, aim: Aim::Direction([0.0, 0.0, 1.0]) }), Ok(()));
        run(&mut m, 0.5);
        assert!(m.unit(target).unwrap().hp < hp, "the generic effect found no target");
    }
}
