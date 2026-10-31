use crate::combat::{DamageType, UnitId};
use crate::sim::{DT, Match};

#[derive(Clone, Debug, PartialEq)]
pub enum ModKind {
    Stun,
    Silence,
    Root,
    Slow(f32),
    Taunt(UnitId),
    Armor(f32),
    MoveSpeed(f32),
    DamageAmp(f32),
    Regen(f32),
    Dot { per_second: f32, kind: DamageType },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Modifier {
    pub owner: UnitId,
    pub name: String,
    pub kind: ModKind,
    pub source: Option<UnitId>,
    pub remaining: f32,
    pub clock: f32,
}

impl Match {
    pub fn add_modifier(&mut self, owner: UnitId, name: &str, kind: ModKind, source: Option<UnitId>, duration: f32) {
        if duration <= 0.0 {
            return;
        }
        if let Some(existing) = self
            .modifiers
            .iter_mut()
            .find(|m| m.owner == owner && m.name == name && m.source == source)
        {
            existing.remaining = duration;
            existing.kind = kind;
            return;
        }
        self.modifiers.push(Modifier {
            owner,
            name: name.to_string(),
            kind,
            source,
            remaining: duration,
            clock: 0.0,
        });
    }

    pub fn modifiers_of(&self, owner: UnitId) -> impl Iterator<Item = &Modifier> {
        self.modifiers.iter().filter(move |m| m.owner == owner)
    }

    pub fn stunned(&self, id: UnitId) -> bool {
        self.modifiers_of(id).any(|m| m.kind == ModKind::Stun)
    }

    pub fn silenced(&self, id: UnitId) -> bool {
        self.modifiers_of(id).any(|m| matches!(m.kind, ModKind::Silence | ModKind::Stun))
    }

    pub fn rooted(&self, id: UnitId) -> bool {
        self.modifiers_of(id).any(|m| matches!(m.kind, ModKind::Root | ModKind::Stun))
    }

    pub fn taunted_by(&self, id: UnitId) -> Option<UnitId> {
        self.modifiers_of(id).find_map(|m| match m.kind {
            ModKind::Taunt(source) => Some(source),
            _ => None,
        })
    }

    pub fn armor_bonus(&self, id: UnitId) -> f32 {
        self.modifiers_of(id)
            .filter_map(|m| match m.kind {
                ModKind::Armor(v) => Some(v),
                _ => None,
            })
            .sum()
    }

    pub fn speed_factor(&self, id: UnitId) -> f32 {
        if self.rooted(id) {
            return 0.0;
        }
        let mut factor = 1.0;
        let mut slow: f32 = 0.0;
        for m in self.modifiers_of(id) {
            match m.kind {
                ModKind::Slow(v) => slow = slow.max(v),
                ModKind::MoveSpeed(v) => factor += v,
                _ => {}
            }
        }
        (factor * (1.0 - slow.clamp(0.0, 1.0))).max(0.0)
    }

    pub(crate) fn step_modifiers(&mut self) {
        let mut dots: Vec<(UnitId, Option<UnitId>, f32, DamageType)> = Vec::new();
        for m in &mut self.modifiers {
            m.remaining -= DT;
            if let ModKind::Dot { per_second, kind } = m.kind {
                m.clock += DT;
                while m.clock >= 1.0 {
                    m.clock -= 1.0;
                    dots.push((m.owner, m.source, per_second, kind));
                }
            }
        }
        self.step_regen_modifiers();
        self.modifiers.retain(|m| m.remaining > 0.0);
        for (owner, source, amount, kind) in dots {
            self.deal_damage(source, owner, amount, kind);
        }
        let alive: Vec<UnitId> = self.all_combatants().into_iter().filter(|c| c.alive).map(|c| c.id).collect();
        self.modifiers.retain(|m| alive.contains(&m.owner));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Data, dev_data_dir};
    use crate::sim::{Rules, Team, TICK_RATE};
    use std::sync::Arc;

    fn arena() -> Option<Match> {
        let data = Arc::new(Data::load(&dev_data_dir()?).ok()?);
        let mut m = Match::new(data, Rules::default());
        m.add_hero(1, Team::Radiant, "npc_dota_hero_axe", [0.0; 3]).ok()?;
        Some(m)
    }

    fn run(m: &mut Match, seconds: f32) {
        for _ in 0..(seconds * TICK_RATE as f32) as u32 {
            m.step();
        }
    }

    #[test]
    fn a_modifier_lives_for_its_duration_and_refreshes_instead_of_stacking() {
        let Some(mut m) = arena() else { return };
        m.add_modifier(1, "stun", ModKind::Stun, None, 2.0);
        m.add_modifier(1, "stun", ModKind::Stun, None, 2.0);
        assert_eq!(m.modifiers_of(1).count(), 1);
        assert!(m.stunned(1));
        run(&mut m, 1.0);
        assert!(m.stunned(1));
        run(&mut m, 1.2);
        assert!(!m.stunned(1));
    }

    #[test]
    fn slows_take_the_strongest_and_speed_bonuses_add() {
        let Some(mut m) = arena() else { return };
        m.add_modifier(1, "a", ModKind::Slow(0.2), None, 5.0);
        m.add_modifier(1, "b", ModKind::Slow(0.5), None, 5.0);
        m.add_modifier(1, "c", ModKind::MoveSpeed(0.2), None, 5.0);
        assert!((m.speed_factor(1) - 1.2 * 0.5).abs() < 1e-5);
        m.add_modifier(1, "root", ModKind::Root, None, 5.0);
        assert_eq!(m.speed_factor(1), 0.0);
    }

    #[test]
    fn a_damage_over_time_ticks_once_a_second_and_stops_with_the_modifier() {
        let Some(mut m) = arena() else { return };
        let creep = m.spawn_unit("npc_dota_creep_badguys_melee", Team::Dire, [30.0, 0.0, 0.0]).unwrap();
        m.units.iter_mut().find(|u| u.id == creep).unwrap().hp_regen = 0.0;
        let hp = m.unit(creep).unwrap().hp;
        m.add_modifier(creep, "burn", ModKind::Dot { per_second: 20.0, kind: DamageType::Pure }, None, 3.0);
        run(&mut m, 3.5);
        let lost = hp - m.unit(creep).unwrap().hp;
        assert!((lost - 60.0).abs() < 0.5, "{lost}");
        assert_eq!(m.modifiers_of(creep).count(), 0);
    }

    #[test]
    fn armor_modifiers_add_up_and_a_death_clears_them() {
        let Some(mut m) = arena() else { return };
        m.add_modifier(1, "x", ModKind::Armor(5.0), None, 60.0);
        m.add_modifier(1, "y", ModKind::Armor(3.0), None, 60.0);
        assert_eq!(m.armor_bonus(1), 8.0);
        m.damage(1, 99999.0);
        run(&mut m, 0.2);
        assert_eq!(m.modifiers_of(1).count(), 0);
    }
}
