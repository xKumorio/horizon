use crate::sim::Team;
use serde_json::Value;
use std::{collections::HashSet, path::Path};

pub const OVERVIEW_ORIGIN: [f32; 2] = [-9472.0, 9472.0];
pub const OVERVIEW_SPAN: f32 = 18944.0;
const WORLD_OFFSET: [f32; 3] = [-56.48959, 193.49749, -0.270934];
const INCH: f32 = 0.0254;

pub fn minimap_uv(pos: [f32; 3]) -> [f32; 2] {
    let source_x = (pos[2] - WORLD_OFFSET[2]) / INCH;
    let source_y = (pos[0] - WORLD_OFFSET[0]) / INCH;
    [
        (source_x - OVERVIEW_ORIGIN[0]) / OVERVIEW_SPAN,
        (OVERVIEW_ORIGIN[1] - source_y) / OVERVIEW_SPAN,
    ]
}

pub const SHOP_HOME: u8 = 1;
pub const SHOP_SECRET: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lane {
    Top,
    Mid,
    Bot,
}

impl Lane {
    pub const ALL: [Lane; 3] = [Lane::Top, Lane::Mid, Lane::Bot];

    pub fn name(self) -> &'static str {
        match self {
            Lane::Top => "top",
            Lane::Mid => "mid",
            Lane::Bot => "bot",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapEntity {
    pub class: String,
    pub name: String,
    pub unit: String,
    pub target: String,
    pub pos: [f32; 3],
    pub yaw: f32,
    pub shoptype: i32,
}

#[derive(Clone, Debug, Default)]
pub struct MapData {
    pub entities: Vec<MapEntity>,
}

fn team_of(text: &str) -> Option<Team> {
    if text.contains("goodguys") || text.ends_with("_good") {
        Some(Team::Radiant)
    } else if text.contains("badguys") || text.ends_with("_bad") {
        Some(Team::Dire)
    } else {
        None
    }
}

impl MapData {
    pub fn load(path: &Path) -> Result<MapData, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let json: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let array = json.as_array().ok_or("not an array")?;
        let mut entities = Vec::new();
        for item in array {
            let str = |k: &str| item.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let pos = item.get("pos").and_then(Value::as_array).ok_or("no pos")?;
            let p: Vec<f32> = pos.iter().filter_map(|v| v.as_f64()).map(|v| v as f32).collect();
            if p.len() < 3 {
                return Err("short pos".into());
            }
            entities.push(MapEntity {
                class: str("class"),
                name: str("targetname"),
                unit: str("mapunitname"),
                target: str("target"),
                pos: [p[0], p[1], p[2]],
                yaw: item.get("yaw").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                shoptype: str("shoptype").parse().unwrap_or(0),
            });
        }
        Ok(MapData { entities })
    }

    pub fn of_class<'a>(&'a self, class: &'a str) -> impl Iterator<Item = &'a MapEntity> + 'a {
        self.entities.iter().filter(move |e| e.class == class)
    }

    pub fn spawn_points(&self, team: Team) -> Vec<[f32; 3]> {
        let class = match team {
            Team::Radiant => "info_player_start_goodguys",
            Team::Dire => "info_player_start_badguys",
        };
        self.of_class(class).map(|e| e.pos).collect()
    }

    pub fn fountain(&self, team: Team) -> Option<[f32; 3]> {
        self.of_class("ent_dota_fountain").find(|e| team_of(&e.name) == Some(team)).map(|e| e.pos)
    }

    pub fn fort(&self, team: Team) -> Option<&MapEntity> {
        self.of_class("npc_dota_fort").find(|e| team_of(&e.name) == Some(team))
    }

    pub fn lane(&self, lane: Lane, team: Team) -> Vec<[f32; 3]> {
        let side = if team == Team::Radiant { "goodguys" } else { "badguys" };
        let prefix = format!("lane_{}_pathcorner_{side}_", lane.name());
        let find = |name: &str| self.of_class("path_corner").find(|e| e.name == name);
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut next = format!("{prefix}1");
        while let Some(node) = find(&next) {
            if !seen.insert(node.name.clone()) {
                break;
            }
            out.push(node.pos);
            if node.target.is_empty() {
                break;
            }
            next = node.target.clone();
        }
        out
    }

    pub fn shop_access(&self, pos: [f32; 3], team: Team, radius: f32) -> u8 {
        let mut access = 0;
        let near = |p: [f32; 3]| crate::combat::distance(p, pos) <= radius;
        for shop in self.of_class("ent_dota_shop").chain(self.of_class("trigger_shop")) {
            if !near(shop.pos) {
                continue;
            }
            match shop.shoptype {
                2 => access |= SHOP_SECRET,
                _ => {
                    let own = self.fountain(team).is_some_and(|f| {
                        let other = self.fountain(team.enemy()).unwrap_or(f);
                        crate::combat::distance(f, shop.pos) <= crate::combat::distance(other, shop.pos)
                    });
                    if own {
                        access |= SHOP_HOME;
                    }
                }
            }
        }
        access
    }

    pub fn buildings(&self, team: Team) -> Vec<&MapEntity> {
        self.entities
            .iter()
            .filter(|e| matches!(e.class.as_str(), "npc_dota_tower" | "npc_dota_barracks" | "npc_dota_fort"))
            .filter(|e| team_of(&e.unit) == Some(team) || team_of(&e.name) == Some(team))
            .collect()
    }
}

pub fn find_map_file() -> Option<std::path::PathBuf> {
    crate::data::find_data_dir().map(|d| d.join("map_entities.json")).filter(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> Option<MapData> {
        MapData::load(&find_map_file()?).ok()
    }

    #[test]
    fn every_lane_is_a_chain_from_base_to_base() {
        let Some(map) = map() else { return };
        for team in [Team::Radiant, Team::Dire] {
            let base = map.fort(team).unwrap().pos;
            let enemy = map.fort(team.enemy()).unwrap().pos;
            for lane in Lane::ALL {
                let path = map.lane(lane, team);
                assert!(path.len() >= 5, "{lane:?} {team:?} {}", path.len());
                let first = crate::combat::distance(path[0], base);
                let last = crate::combat::distance(*path.last().unwrap(), enemy);
                assert!(first < 90.0, "{lane:?} {team:?} starts {first} m from its own base");
                assert!(last < 90.0, "{lane:?} {team:?} ends {last} m from the enemy base");
                assert!(first < crate::combat::distance(path[0], enemy));
            }
        }
    }

    #[test]
    fn each_team_has_its_towers_barracks_and_ancient() {
        let Some(map) = map() else { return };
        for team in [Team::Radiant, Team::Dire] {
            let all = map.buildings(team);
            let towers = all.iter().filter(|e| e.class == "npc_dota_tower").count();
            let rax = all.iter().filter(|e| e.class == "npc_dota_barracks").count();
            let forts = all.iter().filter(|e| e.class == "npc_dota_fort").count();
            assert_eq!((towers, rax, forts), (11, 6, 1), "{team:?}");
            assert!(!map.spawn_points(team).is_empty());
            assert!(map.fountain(team).is_some());
        }
    }

    #[test]
    fn the_two_fountains_sit_in_opposite_corners_of_the_overview() {
        let Some(map) = map() else { return };
        let radiant = minimap_uv(map.fountain(Team::Radiant).unwrap());
        let dire = minimap_uv(map.fountain(Team::Dire).unwrap());
        assert!(radiant[0] < 0.35 && radiant[1] > 0.65, "{radiant:?}");
        assert!(dire[0] > 0.65 && dire[1] < 0.35, "{dire:?}");
        for p in [radiant, dire] {
            assert!((0.0..=1.0).contains(&p[0]) && (0.0..=1.0).contains(&p[1]));
        }
    }

    #[test]
    fn shops_are_reached_by_position() {
        let Some(map) = map() else { return };
        let home = map.fountain(Team::Radiant).unwrap();
        assert_eq!(map.shop_access(home, Team::Radiant, 40.0) & SHOP_HOME, SHOP_HOME);
        assert_eq!(map.shop_access(home, Team::Dire, 40.0) & SHOP_HOME, 0);
        let secret = map.of_class("ent_dota_shop").find(|e| e.shoptype == 2).unwrap().pos;
        assert_eq!(map.shop_access(secret, Team::Radiant, 20.0) & SHOP_SECRET, SHOP_SECRET);
        assert_eq!(map.shop_access([0.0, 0.0, 0.0], Team::Radiant, 20.0), 0);
    }
}
