use crate::ai::UnitKind;
use crate::combat::{UnitId, distance_xz};
use crate::modifiers::ModKind;
use crate::sim::{DT, Match, PlayerId, Reject, Team};

pub const STASH_SLOTS: usize = 6;
const ARRIVE: f32 = 3.0;
const RESPAWN: f32 = 180.0;
const BURST_COOLDOWN: f32 = 120.0;
const BURST_DURATION: f32 = 6.0;
const BURST_SPEED: f32 = 0.5;
const STASH_RANGE: f32 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CourierTask {
    Idle,
    Deliver(PlayerId),
    Home,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CourierCommand {
    Deliver,
    Burst,
    Home,
}

#[derive(Clone, Debug)]
pub struct CourierState {
    pub unit: Option<UnitId>,
    pub task: CourierTask,
    pub respawn: Option<f32>,
    pub burst_cooldown: f32,
}

impl Match {
    pub fn spawn_couriers(&mut self) {
        for team in [Team::Radiant, Team::Dire] {
            self.couriers.insert(
                team,
                CourierState {
                    unit: None,
                    task: CourierTask::Idle,
                    respawn: Some(0.0),
                    burst_cooldown: 0.0,
                },
            );
        }
    }

    fn make_courier(&mut self, team: Team) -> Option<UnitId> {
        let home = self.spawn_positions.get(&team).copied()?;
        let id = self.spawn_unit("npc_dota_courier", team, home)?;
        let u = self.units.iter_mut().find(|u| u.id == id)?;
        u.kind = UnitKind::Courier;
        u.ai = false;
        if u.max_hp < 1.0 {
            u.max_hp = 75.0;
            u.hp = 75.0;
        }
        u.hp_regen = 0.0;
        Some(id)
    }

    pub fn courier_of(&self, team: Team) -> Option<&crate::combat::Unit> {
        let id = self.couriers.get(&team)?.unit?;
        self.unit(id).filter(|u| u.alive)
    }

    pub fn courier_command(&mut self, player: PlayerId, command: CourierCommand) -> Result<(), Reject> {
        let team = self.hero(player).ok_or(Reject::UnknownPlayer)?.team;
        let Some(unit) = self.courier_of(team).map(|u| u.id) else {
            return Err(Reject::NoCourier);
        };
        match command {
            CourierCommand::Deliver => {
                if let Some(c) = self.couriers.get_mut(&team) {
                    c.task = CourierTask::Deliver(player);
                }
                Ok(())
            }
            CourierCommand::Home => {
                if let Some(c) = self.couriers.get_mut(&team) {
                    c.task = CourierTask::Home;
                }
                Ok(())
            }
            CourierCommand::Burst => {
                let ready = self.couriers.get(&team).is_some_and(|c| c.burst_cooldown <= 0.0);
                if !ready {
                    return Err(Reject::OnCooldown);
                }
                self.add_modifier(unit, "modifier_courier_burst", ModKind::MoveSpeed(BURST_SPEED), None, BURST_DURATION);
                if let Some(c) = self.couriers.get_mut(&team) {
                    c.burst_cooldown = BURST_COOLDOWN;
                }
                Ok(())
            }
        }
    }

    pub fn move_stash_to_bag(&mut self, player: PlayerId) -> usize {
        let Some(h) = self.heroes.iter_mut().find(|h| h.player == player) else { return 0 };
        let mut moved = 0;
        for s in 0..STASH_SLOTS {
            if h.stash[s].is_none() {
                continue;
            }
            let Some(free) = h.free_slot() else { break };
            h.items[free] = h.stash[s].take();
            moved += 1;
        }
        moved
    }

    pub(crate) fn step_couriers(&mut self) {
        let teams = [Team::Radiant, Team::Dire];
        for team in teams {
            let Some(mut state) = self.couriers.get(&team).cloned() else { continue };
            state.burst_cooldown = (state.burst_cooldown - DT).max(0.0);
            let alive = state.unit.and_then(|id| self.unit(id)).is_some_and(|u| u.alive);
            if !alive {
                if state.unit.is_some() {
                    state.unit = None;
                    state.task = CourierTask::Idle;
                    state.respawn = Some(RESPAWN);
                }
                if let Some(left) = state.respawn {
                    let left = left - DT;
                    if left <= 0.0 {
                        state.unit = self.make_courier(team);
                        state.respawn = None;
                    } else {
                        state.respawn = Some(left);
                    }
                }
                self.couriers.insert(team, state);
                continue;
            }
            let id = state.unit.unwrap_or(0);
            let home = self.spawn_positions.get(&team).copied().unwrap_or([0.0; 3]);
            match state.task {
                CourierTask::Idle => {}
                CourierTask::Deliver(player) => match self.hero(player).filter(|h| h.alive).map(|h| h.pos) {
                    Some(goal) => {
                        if self.step_toward(id, goal, ARRIVE) {
                            self.move_stash_to_bag(player);
                            state.task = CourierTask::Home;
                        }
                    }
                    None => state.task = CourierTask::Home,
                },
                CourierTask::Home => {
                    if self.step_toward(id, home, 1.0) {
                        state.task = CourierTask::Idle;
                    }
                }
            }
            self.couriers.insert(team, state);
        }
        let near: Vec<PlayerId> = self
            .heroes
            .iter()
            .filter(|h| h.alive)
            .filter(|h| self.spawn_positions.get(&h.team).is_some_and(|p| distance_xz(*p, h.pos) < STASH_RANGE))
            .map(|h| h.player)
            .collect();
        for p in near {
            self.move_stash_to_bag(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Data, dev_data_dir};
    use crate::map::{MapData, find_map_file};
    use crate::sim::{Intent, Rules, TICK_RATE};
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

    fn far_hero(m: &mut Match) {
        let home = m.spawn_positions[&Team::Radiant];
        m.add_hero(1, Team::Radiant, "npc_dota_hero_axe", [home[0] + 90.0, home[1], home[2] + 60.0]).unwrap();
        m.heroes[0].gold = 100000;
    }

    #[test]
    fn each_team_gets_a_courier_at_its_base_with_the_speed_from_the_files() {
        let Some(mut m) = world() else { return };
        run(&mut m, 0.2);
        for team in [Team::Radiant, Team::Dire] {
            let c = m.courier_of(team).unwrap();
            assert_eq!(c.move_speed, 290.0);
            assert!(crate::combat::distance(c.pos, m.spawn_positions[&team]) < 0.5);
        }
    }

    #[test]
    fn a_purchase_away_from_the_shop_waits_in_the_stash() {
        let Some(mut m) = world() else { return };
        far_hero(&mut m);
        let gold = m.hero(1).unwrap().gold;
        let buy = Intent::Buy { item: "item_tango".into(), at_shop: false };
        assert_eq!(m.submit(1, buy.clone()), Ok(()));
        let h = m.hero(1).unwrap();
        assert!(h.stash.iter().flatten().any(|i| i.name == "item_tango"));
        assert!(!h.owns("item_tango"));
        assert_eq!(h.gold, gold - m.item_cost("item_tango").unwrap());
        for _ in 0..5 {
            m.submit(1, buy.clone()).unwrap();
        }
        assert_eq!(m.submit(1, buy), Err(Reject::StashFull));
    }

    #[test]
    fn the_courier_carries_the_stash_to_the_hero_and_goes_home() {
        let Some(mut m) = world() else { return };
        far_hero(&mut m);
        m.submit(1, Intent::Buy { item: "item_tango".into(), at_shop: false }).unwrap();
        m.submit(1, Intent::Buy { item: "item_clarity".into(), at_shop: false }).unwrap();
        run(&mut m, 0.2);
        assert_eq!(m.courier_command(1, CourierCommand::Deliver), Ok(()));
        run(&mut m, 8.0);
        let courier_pos = m.courier_of(Team::Radiant).unwrap().pos;
        assert!(distance_xz(courier_pos, m.hero(1).unwrap().pos) < 60.0, "it is on its way");
        run(&mut m, 30.0);
        let h = m.hero(1).unwrap();
        assert!(h.owns("item_tango") && h.owns("item_clarity"), "delivered");
        assert!(h.stash.iter().flatten().count() == 0);
        run(&mut m, 40.0);
        let home = m.spawn_positions[&Team::Radiant];
        assert!(distance_xz(m.courier_of(Team::Radiant).unwrap().pos, home) < 2.0, "and home again");
    }

    #[test]
    fn walking_back_to_the_fountain_empties_the_stash_by_itself() {
        let Some(mut m) = world() else { return };
        far_hero(&mut m);
        m.submit(1, Intent::Buy { item: "item_tango".into(), at_shop: false }).unwrap();
        let home = m.spawn_positions[&Team::Radiant];
        m.heroes[0].pos = [home[0] + 5.0, home[1], home[2]];
        run(&mut m, 0.2);
        assert!(m.hero(1).unwrap().owns("item_tango"));
    }

    #[test]
    fn burst_speeds_the_courier_up_then_needs_its_cooldown() {
        let Some(mut m) = world() else { return };
        far_hero(&mut m);
        run(&mut m, 0.2);
        let id = m.courier_of(Team::Radiant).unwrap().id;
        assert_eq!(m.speed_factor(id), 1.0);
        assert_eq!(m.courier_command(1, CourierCommand::Burst), Ok(()));
        assert!((m.speed_factor(id) - 1.5).abs() < 1e-5);
        assert_eq!(m.courier_command(1, CourierCommand::Burst), Err(Reject::OnCooldown));
        run(&mut m, 6.5);
        assert_eq!(m.speed_factor(id), 1.0);
        run(&mut m, 115.0);
        assert_eq!(m.courier_command(1, CourierCommand::Burst), Ok(()));
    }

    #[test]
    fn a_killed_courier_comes_back_after_its_timer() {
        let Some(mut m) = world() else { return };
        far_hero(&mut m);
        run(&mut m, 0.2);
        let id = m.courier_of(Team::Radiant).unwrap().id;
        m.deal_damage(None, id, 9999.0, crate::combat::DamageType::Pure);
        run(&mut m, 0.2);
        assert!(m.courier_of(Team::Radiant).is_none());
        assert_eq!(m.courier_command(1, CourierCommand::Deliver), Err(Reject::NoCourier));
        run(&mut m, 181.0);
        assert!(m.courier_of(Team::Radiant).is_some());
    }

    #[test]
    fn lane_creeps_and_towers_do_not_shoot_couriers() {
        let Some(mut m) = world() else { return };
        far_hero(&mut m);
        run(&mut m, 0.2);
        let c = m.courier_of(Team::Radiant).unwrap().clone();
        let creep = m.spawn_unit("npc_dota_creep_badguys_melee", Team::Dire, [c.pos[0] + 1.0, c.pos[1], c.pos[2]]).unwrap();
        m.units.iter_mut().find(|u| u.id == creep).unwrap().ai = true;
        run(&mut m, 4.0);
        let after = m.courier_of(Team::Radiant).unwrap();
        assert_eq!(after.hp, after.max_hp);
    }
}
