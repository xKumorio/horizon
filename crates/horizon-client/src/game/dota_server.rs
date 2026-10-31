use super::{Game, Options, lan::LanHost};
use horizon_dota::{
    data::{Data, find_data_dir},
    bots::BOT_FIRST_ID,
    map::{Lane, MapData, SHOP_HOME, SHOP_SECRET, find_map_file},
    sim::{DT, Intent, Match, Rules, Team},
    wire,
};
use horizon_core::geometry::Vec3;
use horizon_dota::{ai::UnitKind, combat::UnitId};
use horizon_protocol::packets::{configuration::CustomPayload, play};
use std::{collections::HashMap, sync::Arc};

const DEFAULT_HERO: &str = "npc_dota_hero_axe";
const UNIT_METERS: f32 = 0.0254;
const SEND_EVERY: u32 = 2;
const TICK_SECONDS: f32 = 0.05;
const SHOP_REACH: f32 = 30.0;

struct Seat {
    player: u32,
    team: Team,
    home: [f64; 3],
    hero: bool,
}

pub(super) struct DotaServer {
    m: Match,
    map: Option<MapData>,
    seats: HashMap<u32, Seat>,
    next_player: u32,
    hero: String,
    carry: f32,
    ticks: u32,
    visual: HashMap<UnitId, u32>,
    difficulty_set: bool,
    pick_required: bool,
    speed: f32,
}

impl DotaServer {
    pub(super) fn load(options: &Options) -> Option<DotaServer> {
        if !options.dota {
            return None;
        }
        let root = find_data_dir()?;
        let data = match Data::load(&root) {
            Ok(data) => Arc::new(data),
            Err(e) => {
                log::error!("Dota data in {}: {e}", root.display());
                return None;
            }
        };
        let hero = options
            .dota_hero
            .clone()
            .unwrap_or_else(|| DEFAULT_HERO.to_string());
        if data.hero(&hero).is_none() {
            log::error!("Dota: unknown hero {hero}");
            return None;
        }
        let map = find_map_file().and_then(|p| match MapData::load(&p) {
            Ok(map) => Some(map),
            Err(e) => {
                log::error!("Dota map entities: {e}");
                None
            }
        });
        log::info!(
            "Dota mode: {} heroes, {} items, {} map objects",
            data.heroes.len(),
            data.items.len(),
            map.as_ref().map_or(0, |m| m.entities.len())
        );
        let mut m = Match::new(data, Rules::default());
        if let Some(map) = &map {
            m.setup_map(map);
            if std::env::var_os("HORIZON_DOTA_NO_BOTS").is_none() {
                add_bots(&mut m);
            }
        }
        Some(DotaServer {
            m,
            map,
            seats: HashMap::new(),
            next_player: 1,
            hero,
            carry: 0.0,
            ticks: 0,
            visual: HashMap::new(),
            difficulty_set: false,
            pick_required: std::env::var_os("HORIZON_DOTA_NO_PICK").is_none() && options.dota_hero.is_none(),
            speed: std::env::var("HORIZON_DOTA_SPEED").ok().and_then(|v| v.parse::<f32>().ok()).map_or(1.0, |v| v.clamp(1.0, 40.0)),
        })
    }

    fn spawn_seat_hero(&mut self, conn: u32, name: &str) -> Result<(), String> {
        let seat = self.seats.get(&conn).ok_or("no seat")?;
        let (player, team, home) = (seat.player, seat.team, seat.home);
        let pos = [home[0] as f32, home[1] as f32, home[2] as f32];
        self.m.add_hero(player, team, name, pos)?;
        if let Some(s) = self.seats.get_mut(&conn) {
            s.hero = true;
        }
        Ok(())
    }

    fn hero_is_free(&self, name: &str) -> bool {
        self.m.data.hero(name).is_some_and(|h| h.raw.str("Enabled") == Some("1"))
            && !self.m.heroes.iter().any(|h| h.def.name == name)
    }

    fn access(&self, conn: u32, position: [f64; 3]) -> u8 {
        let Some(seat) = self.seats.get(&conn) else {
            return 0;
        };
        let pos = [position[0] as f32, position[1] as f32, position[2] as f32];
        match &self.map {
            Some(map) => map.shop_access(pos, seat.team, SHOP_REACH),
            None => {
                let radius = f64::from(self.m.rules.shop_radius * UNIT_METERS);
                let d = [
                    position[0] - seat.home[0],
                    position[1] - seat.home[1],
                    position[2] - seat.home[2],
                ];
                if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= radius * radius {
                    SHOP_HOME | SHOP_SECRET
                } else {
                    0
                }
            }
        }
    }

    fn is_secret(&self, item: &str) -> bool {
        self.m
            .data
            .shop("secretshop")
            .is_some_and(|s| s.items.iter().any(|i| i == item))
    }
}

fn add_bots(m: &mut Match) {
    let radiant = [
        ("npc_dota_hero_sven", Lane::Top),
        ("npc_dota_hero_crystal_maiden", Lane::Bot),
        ("npc_dota_hero_lina", Lane::Bot),
        ("npc_dota_hero_juggernaut", Lane::Top),
    ];
    let dire = [
        ("npc_dota_hero_pudge", Lane::Mid),
        ("npc_dota_hero_drow_ranger", Lane::Top),
        ("npc_dota_hero_zuus", Lane::Bot),
        ("npc_dota_hero_earthshaker", Lane::Bot),
        ("npc_dota_hero_sand_king", Lane::Top),
    ];
    for (team, roster, base) in [(Team::Radiant, &radiant[..], 1), (Team::Dire, &dire[..], 11)] {
        let Some(start) = m.spawn_positions.get(&team).copied() else { continue };
        for (i, (name, lane)) in roster.iter().enumerate() {
            if let Err(e) = m.add_bot(BOT_FIRST_ID + base + i as u32, team, name, *lane, start) {
                log::warn!("Dota bot {name}: {e}");
            }
        }
    }
}

fn visual_kind(name: &str, kind: UnitKind, team: Team) -> &'static str {
    if name.starts_with("npc_dota_hero_") {
        return if team == Team::Radiant { "minecraft:vindicator" } else { "minecraft:piglin_brute" };
    }
    if name.starts_with("rune_") {
        return "minecraft:allay";
    }
    match (kind, team) {
        (UnitKind::Tower, _) => "minecraft:iron_golem",
        (UnitKind::Barracks, _) => "minecraft:ravager",
        (UnitKind::Fort, _) => "minecraft:ravager",
        (UnitKind::Siege, _) => "minecraft:ravager",
        (UnitKind::Neutral, _) => "minecraft:ravager",
        (UnitKind::Courier, _) => "minecraft:parrot",
        (_, Team::Radiant) if name.contains("ranged") => "minecraft:skeleton",
        (_, Team::Radiant) => "minecraft:zombie",
        (_, Team::Dire) if name.contains("ranged") => "minecraft:stray",
        (_, Team::Dire) => "minecraft:husk",
    }
}

fn send(host: &LanHost, conn: u32, data: Vec<u8>) {
    host.hub.send_to(
        conn,
        &play::Clientbound::CustomPayload(CustomPayload {
            channel: wire::CHANNEL.into(),
            data,
        }),
    );
}

impl Game<'_> {
    pub(super) fn dota_joined(&mut self, host: &mut LanHost, conn: u32) {
        let Some(index) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let current = host.hub.guests[index].position;
        let Some(d) = host.dota.as_mut() else { return };
        if d.seats.contains_key(&conn) {
            return;
        }
        let player = d.next_player;
        d.next_player += 1;
        let team = if player % 2 == 1 {
            Team::Radiant
        } else {
            Team::Dire
        };
        let start = d.map.as_ref().and_then(|m| {
            let points = m.spawn_points(team);
            points.get((player as usize / 2) % points.len().max(1)).copied()
        });
        let position = start.map_or(current, |p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]);
        let pos = [position[0] as f32, position[1] as f32, position[2] as f32];
        let _ = pos;
        let pick = d.pick_required;
        d.seats.insert(
            conn,
            Seat {
                player,
                team,
                home: position,
                hero: false,
            },
        );
        if pick {
            let taken: Vec<String> = d.m.heroes.iter().map(|h| h.def.name.clone()).collect();
            send(host, conn, wire::encode_pick(&taken));
        } else {
            let name = d.hero.clone();
            if let Err(e) = d.spawn_seat_hero(conn, &name) {
                log::error!("Dota: {e}");
                return;
            }
            let access = d.access(conn, position);
            let bytes = d
                .m
                .hero(player)
                .map(|hero| wire::encode_snapshot(&wire::snapshot_for(&d.m, hero, access != 0)));
            if let Some(bytes) = bytes {
                send(host, conn, bytes);
            }
        }
        if start.is_some() {
            host.hub.teleport(index, Some(position));
        }
    }

    pub(super) fn dota_left(&mut self, host: &mut LanHost, conn: u32) {
        let Some(d) = host.dota.as_mut() else { return };
        if let Some(seat) = d.seats.remove(&conn) {
            d.m.heroes.retain(|h| h.player != seat.player);
        }
    }

    pub(super) fn dota_payload(&mut self, host: &mut LanHost, conn: u32, data: &[u8]) {
        let Some(position) = host.hub.guest(conn).map(|g| g.position) else {
            return;
        };
        let Some(d) = host.dota.as_mut() else { return };
        let Some(player) = d.seats.get(&conn).map(|s| s.player) else {
            return;
        };
        let access = d.access(conn, position);
        if let Ok(Intent::PickHero(name)) = wire::decode_intent(data, false) {
            let has_hero = d.seats.get(&conn).is_some_and(|s| s.hero);
            let reply = if has_hero {
                None
            } else if d.hero_is_free(&name) && d.spawn_seat_hero(conn, &name).is_ok() {
                d.m.hero(player)
                    .map(|hero| wire::encode_snapshot(&wire::snapshot_for(&d.m, hero, access != 0)))
            } else {
                let taken: Vec<String> = d.m.heroes.iter().map(|h| h.def.name.clone()).collect();
                Some(wire::encode_pick(&taken))
            };
            if let Some(bytes) = reply {
                send(host, conn, bytes);
            }
            return;
        }
        match wire::decode_intent(data, access != 0) {
            Ok(intent) => {
                if let Intent::Buy { item, .. } = &intent
                    && d.is_secret(item)
                    && access & SHOP_SECRET == 0
                {
                    log::info!("Dota: player {player} cannot buy {item} away from the secret shop");
                    return;
                }
                let intent = match intent {
                    Intent::Buy { item, .. } => {
                        let at_shop = access & SHOP_HOME != 0 || (d.is_secret(&item) && access & SHOP_SECRET != 0);
                        Intent::Buy { item, at_shop }
                    }
                    Intent::Sell { slot, .. } => Intent::Sell { slot, at_shop: access != 0 },
                    other => other,
                };
                let attack = matches!(intent, Intent::Attack { .. });
                match d.m.submit(player, intent.clone()) {
                    Err(reject) => log::info!("Dota: player {player} {intent:?} refused: {reject:?}"),
                    Ok(()) if attack => log::info!("Dota: player {player} attack started"),
                    Ok(()) => {}
                }
            }
            Err(e) => log::warn!("Dota: player {player} sent a bad payload: {e}"),
        }
    }

    pub(super) fn dota_tick(&mut self, host: &mut LanHost) {
        let positions: Vec<(u32, [f64; 3])> = match host.dota.as_ref() {
            Some(d) => d
                .seats
                .keys()
                .filter_map(|&c| host.hub.guest(c).map(|g| (c, g.position)))
                .collect(),
            None => return,
        };
        let Some(d) = host.dota.as_mut() else { return };
        for (conn, position) in &positions {
            if let Some(seat) = d.seats.get(conn)
                && let Some(hero) = d.m.heroes.iter_mut().find(|h| h.player == seat.player)
            {
                hero.pos = [position[0] as f32, position[1] as f32, position[2] as f32];
            }
        }
        d.carry += TICK_SECONDS * d.speed;
        while d.carry >= DT {
            d.m.step();
            d.carry -= DT;
        }
        d.ticks += 1;
        let respawned: Vec<u32> = d
            .m
            .events
            .iter()
            .filter_map(|e| match e {
                horizon_dota::sim::Event::Respawned { player } => Some(*player),
                _ => None,
            })
            .collect();
        let back: Vec<(u32, [f64; 3])> = d
            .seats
            .iter()
            .filter(|(_, seat)| respawned.contains(&seat.player))
            .map(|(conn, seat)| (*conn, seat.home))
            .collect();
        for (conn, home) in back {
            if let Some(index) = host.hub.guests.iter().position(|g| g.conn == conn) {
                host.hub.teleport(index, Some(home));
            }
        }
        if host.dota.as_ref().is_some_and(|d| !d.difficulty_set) {
            if let Some(d) = host.dota.as_mut() {
                d.difficulty_set = true;
            }
            if self.ui.difficulty == 0 {
                self.ui.difficulty = 2;
                self.level.set_difficulty(2);
            }
            for rule in ["spawn_mobs", "spawn_monsters", "spawn_patrols", "spawn_wandering_traders"] {
                self.ui.rules.set(rule, horizon_client::gamerules::Value::Bool(false));
            }
            self.sync_rules();
        }
        self.dota_sync_units(host);
        let Some(d) = host.dota.as_mut() else { return };
        if d.ticks % SEND_EVERY != 0 {
            return;
        }
        let sounds: Vec<(String, [f32; 3])> = d
            .m
            .events
            .iter()
            .filter_map(|e| match e {
                horizon_dota::sim::Event::Sound { event, pos } => Some((event.clone(), *pos)),
                _ => None,
            })
            .take(24)
            .collect();
        let listeners: Vec<(u32, [f32; 3])> = d
            .seats
            .iter()
            .filter_map(|(conn, seat)| d.m.hero(seat.player).map(|h| (*conn, h.pos)))
            .collect();
        d.m.drain_events();
        let mut heard: Vec<(u32, Vec<u8>)> = Vec::new();
        for (event, pos) in &sounds {
            for (conn, at) in &listeners {
                let (dx, dz) = (pos[0] - at[0], pos[2] - at[2]);
                if dx * dx + dz * dz <= 80.0 * 80.0 {
                    heard.push((*conn, wire::encode_sound(event, *pos)));
                }
            }
        }
        for (conn, data) in heard {
            send(host, conn, data);
        }
        let Some(d) = host.dota.as_mut() else { return };
        let out: Vec<(u32, Vec<u8>)> = d
            .seats
            .iter()
            .filter_map(|(conn, seat)| {
                let hero = d.m.hero(seat.player)?;
                let near = positions
                    .iter()
                    .find(|(c, _)| c == conn)
                    .is_some_and(|(_, p)| d.access(*conn, *p) != 0);
                Some((*conn, wire::encode_snapshot(&wire::snapshot_for(&d.m, hero, near))))
            })
            .collect();
        for (conn, data) in out {
            send(host, conn, data);
        }
    }
}

impl Game<'_> {
    fn dota_sync_units(&mut self, host: &mut LanHost) {
        struct View {
            id: UnitId,
            name: String,
            kind: UnitKind,
            team: Team,
            pos: [f32; 3],
            fraction: f32,
            alive: bool,
            label: String,
        }
        let Some(d) = host.dota.as_ref() else { return };
        let mut views: Vec<View> = d
            .m
            .units
            .iter()
            .map(|u| View {
                id: u.id,
                name: u.name.clone(),
                kind: u.kind,
                team: u.team,
                pos: u.pos,
                fraction: u.hp / u.max_hp.max(1.0),
                alive: u.alive,
                label: d.m.data.display_name(&u.name),
            })
            .collect();
        for h in d.m.heroes.iter().filter(|h| h.bot.is_some()) {
            let max = h.stats(&d.m.rules).max_hp;
            views.push(View {
                id: h.player,
                name: h.def.name.clone(),
                kind: UnitKind::Creep,
                team: h.team,
                pos: h.pos,
                fraction: h.hp / max.max(1.0),
                alive: h.alive,
                label: format!("{} {}", d.m.data.display_name(&h.def.name), h.level),
            });
        }
        for r in &d.m.rune_state.runes {
            views.push(View {
                id: r.id,
                name: r.kind.key().to_string(),
                kind: UnitKind::Courier,
                team: Team::Radiant,
                pos: [r.pos[0], r.pos[1] + 1.5, r.pos[2]],
                fraction: 1.0,
                alive: true,
                label: r.kind.title().to_string(),
            });
        }
        let known: Vec<UnitId> = views.iter().map(|v| v.id).collect();
        let gone: Vec<(UnitId, u32)> = d
            .visual
            .iter()
            .filter(|(id, _)| !known.contains(id))
            .map(|(id, mob)| (*id, *mob))
            .collect();
        if let Some(d) = host.dota.as_ref() {
            let owned: Vec<u32> = d.visual.values().copied().collect();
            for m in self.mobs.iter_mut() {
                let ours = (m.no_ai && m.no_gravity && m.invulnerable_flag && m.silent && m.name_visible)
                    || m.custom_name.as_deref().is_some_and(|n| n.starts_with('\u{200b}'));
                if ours && !owned.contains(&m.id) {
                    m.removed = true;
                }
            }
        }
        let mut assigned: Vec<(UnitId, u32)> = Vec::new();
        let mut failed = 0;
        for v in &views {
            let existing = host
                .dota
                .as_ref()
                .and_then(|d| d.visual.get(&v.id).copied())
                .filter(|id| self.mobs.iter().any(|m| m.id == *id && !m.removed));
            let mob_id = match existing {
                Some(id) => id,
                None => {
                    if !v.alive {
                        continue;
                    }
                    let kind = visual_kind(&v.name, v.kind, v.team);
                    let Some(id) = self.spawn_mob(kind, [f64::from(v.pos[0]), f64::from(v.pos[1]), f64::from(v.pos[2])], true) else {
                        failed += 1;
                        continue;
                    };
                    assigned.push((v.id, id));
                    if v.kind.is_building() {
                        log::info!("Dota: mob {id} for building unit {} {} at {:?}", v.id, v.name, v.pos);
                    }
                    id
                }
            };
            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob_id) {
                m.no_ai = true;
                m.no_gravity = true;
                m.invulnerable_flag = true;
                m.persistent = true;
                m.silent = true;
                m.name_visible = true;
                m.custom_name = Some(if v.name.starts_with("rune_") {
                    format!("\u{200b}{}", v.label)
                } else {
                    format!("\u{200b}{} {:.0}%", v.label, v.fraction * 100.0)
                });
                m.previous = m.position;
                let ground = horizon_client::dota_map::pick_surface(
                    [f64::from(v.pos[0]), f64::from(v.pos[1]) + 6.0, f64::from(v.pos[2])],
                    [0.0, -1.0, 0.0],
                    24.0,
                )
                .map(|p| p.point[1])
                .unwrap_or(f64::from(v.pos[1]));
                m.position = Vec3::new(f64::from(v.pos[0]), ground, f64::from(v.pos[2]));
                m.needs_sync = true;
                if !v.alive {
                    m.removed = true;
                }
            }
        }
        if let Some(d) = host.dota.as_ref()
            && d.ticks % 200 == 0
        {
            let creeps = d.m.units.iter().filter(|u| u.alive && !u.kind.is_building()).count();
            let towers = d.m.units.iter().filter(|u| u.alive && u.kind.is_building()).count();
            let with_mob = d.visual.len();
            let present = d.visual.values().filter(|id| self.mobs.iter().any(|m| m.id == **id)).count();
            let bot_ids: Vec<(u32, bool)> = d.m.heroes.iter().filter(|h| h.bot.is_some()).map(|h| (h.player, d.visual.get(&h.player).is_some_and(|id| self.mobs.iter().any(|m| m.id == *id)))).collect();
            let bot_mobs: Vec<String> = d
                .m
                .heroes
                .iter()
                .filter(|h| h.bot.is_some())
                .take(3)
                .filter_map(|h| {
                    let id = *d.visual.get(&h.player)?;
                    let m = self.mobs.iter().find(|m| m.id == id)?;
                    Some(format!("{}:{} at ({:.0},{:.0},{:.0}) removed={} no_ai={} sim=({:.0},{:.0},{:.0})", h.player, id, m.position.x, m.position.y, m.position.z, m.removed, m.no_ai, h.pos[0], h.pos[1], h.pos[2]))
                })
                .collect();
            log::info!("Dota mobs: {with_mob} mapped, {present} present, bots {bot_ids:?} {bot_mobs:?}");
            let mobs = self.mobs.len();
            let radiant = d.m.units.iter().filter(|u| u.alive && !u.kind.is_building() && u.team == Team::Radiant).count();
            let dire = d.m.units.iter().filter(|u| u.alive && !u.kind.is_building() && u.team == Team::Dire).count();
            let mid = d.m.units.iter().find(|u| u.alive && u.team == Team::Dire && !u.kind.is_building()).map(|u| u.pos);
            log::info!("Dota units: {creeps} creeps ({radiant} radiant, {dire} dire, first dire at {mid:?}), {towers} buildings, {with_mob} with a mob, {mobs} mobs in the world, {failed} spawns failed, time {:.0}s, waves {}", d.m.time(), d.m.waves_spawned);
        }
        let Some(d) = host.dota.as_mut() else { return };
        for (id, mob) in assigned {
            d.visual.insert(id, mob);
        }
        for v in &views {
            if !v.alive {
                d.visual.remove(&v.id);
            }
        }
        for (id, mob) in gone {
            d.visual.remove(&id);
            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                m.removed = true;
            }
        }
    }
}
