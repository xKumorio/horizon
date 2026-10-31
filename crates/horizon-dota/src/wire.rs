use crate::sim::{Aim, Hero, Intent, Match, Rules, SLOTS, Team, ABILITY_SLOTS};

pub const CHANNEL: &str = "horizon:dota";
pub const MAX_PAYLOAD: usize = 32767;
pub const MAX_NAME: usize = 64;

#[derive(Default)]
pub struct Writer {
    pub bytes: Vec<u8>,
}

impl Writer {
    pub fn u8(&mut self, v: u8) {
        self.bytes.push(v);
    }

    pub fn u16(&mut self, v: u16) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u32(&mut self, v: u32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    pub fn i32(&mut self, v: i32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    pub fn f32(&mut self, v: f32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    pub fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }

    pub fn str(&mut self, v: &str) {
        let v: String = v.chars().take(MAX_NAME).collect();
        self.u8(v.len() as u8);
        self.bytes.extend_from_slice(v.as_bytes());
    }

    pub fn vec3(&mut self, v: [f32; 3]) {
        for c in v {
            self.f32(c);
        }
    }
}

pub struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, at: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).ok_or("overflow")?;
        let out = self.bytes.get(self.at..end).ok_or("truncated")?;
        self.at = end;
        Ok(out)
    }

    pub fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn f32(&mut self) -> Result<f32, String> {
        let v = f32::from_le_bytes(self.take(4)?.try_into().unwrap());
        if v.is_finite() {
            Ok(v)
        } else {
            Err("not finite".into())
        }
    }

    pub fn bool(&mut self) -> Result<bool, String> {
        Ok(self.u8()? != 0)
    }

    pub fn str(&mut self) -> Result<String, String> {
        let n = self.u8()? as usize;
        if n > MAX_NAME * 4 {
            return Err("name too long".into());
        }
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| "bad utf8".to_string())
    }

    pub fn vec3(&mut self) -> Result<[f32; 3], String> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    pub fn done(&self) -> bool {
        self.at == self.bytes.len()
    }
}

fn write_aim(w: &mut Writer, aim: &Aim) {
    match aim {
        Aim::None => w.u8(0),
        Aim::Point(p) => {
            w.u8(1);
            w.vec3(*p);
        }
        Aim::Unit(id) => {
            w.u8(2);
            w.u32(*id);
        }
        Aim::Direction(d) => {
            w.u8(3);
            w.vec3(*d);
        }
    }
}

fn read_aim(r: &mut Reader) -> Result<Aim, String> {
    Ok(match r.u8()? {
        0 => Aim::None,
        1 => Aim::Point(r.vec3()?),
        2 => Aim::Unit(r.u32()?),
        3 => Aim::Direction(r.vec3()?),
        other => return Err(format!("bad aim {other}")),
    })
}

pub fn encode_intent(intent: &Intent) -> Vec<u8> {
    let mut w = Writer::default();
    match intent {
        Intent::LevelUp(slot) => {
            w.u8(0);
            w.u8(*slot as u8);
        }
        Intent::Cast { slot, aim } => {
            w.u8(1);
            w.u8(*slot as u8);
            write_aim(&mut w, aim);
        }
        Intent::Buy { item, .. } => {
            w.u8(2);
            w.str(item);
        }
        Intent::Sell { slot, .. } => {
            w.u8(3);
            w.u8(*slot as u8);
        }
        Intent::UseItem { slot, aim } => {
            w.u8(4);
            w.u8(*slot as u8);
            write_aim(&mut w, aim);
        }
        Intent::SwapItems(a, b) => {
            w.u8(5);
            w.u8(*a as u8);
            w.u8(*b as u8);
        }
        Intent::Attack { dir } => {
            w.u8(6);
            w.vec3(*dir);
        }
        Intent::PickHero(name) => {
            w.u8(7);
            w.str(name);
        }
        Intent::Courier(command) => w.u8(match command {
            crate::courier::CourierCommand::Deliver => 8,
            crate::courier::CourierCommand::Burst => 9,
            crate::courier::CourierCommand::Home => 10,
        }),
    }
    w.bytes
}

pub fn decode_intent(bytes: &[u8], at_shop: bool) -> Result<Intent, String> {
    if bytes.len() > MAX_PAYLOAD {
        return Err("too large".into());
    }
    let mut r = Reader::new(bytes);
    let intent = match r.u8()? {
        0 => Intent::LevelUp(r.u8()? as usize),
        1 => Intent::Cast {
            slot: r.u8()? as usize,
            aim: read_aim(&mut r)?,
        },
        2 => Intent::Buy {
            item: r.str()?,
            at_shop,
        },
        3 => Intent::Sell {
            slot: r.u8()? as usize,
            at_shop,
        },
        4 => Intent::UseItem {
            slot: r.u8()? as usize,
            aim: read_aim(&mut r)?,
        },
        5 => Intent::SwapItems(r.u8()? as usize, r.u8()? as usize),
        6 => Intent::Attack { dir: r.vec3()? },
        7 => Intent::PickHero(r.str()?),
        8 => Intent::Courier(crate::courier::CourierCommand::Deliver),
        9 => Intent::Courier(crate::courier::CourierCommand::Burst),
        10 => Intent::Courier(crate::courier::CourierCommand::Home),
        other => return Err(format!("bad intent {other}")),
    };
    if !r.done() {
        return Err("trailing bytes".into());
    }
    Ok(intent)
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ItemView {
    pub name: String,
    pub charges: i32,
    pub cooldown: f32,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct AbilityView {
    pub name: String,
    pub level: u32,
    pub max_level: u32,
    pub cooldown: f32,
    pub cooldown_total: f32,
    pub mana: f32,
    pub passive: bool,
    pub can_level: bool,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct TeamHeroView {
    pub player: u32,
    pub hero: String,
    pub radiant: bool,
    pub level: u32,
    pub hp_fraction: f32,
    pub alive: bool,
    pub respawn_in: f32,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Marker {
    pub x: f32,
    pub z: f32,
    pub kind: u8,
    pub radiant: bool,
    pub you: bool,
}

pub const MARKER_HERO: u8 = 0;
pub const MARKER_CREEP: u8 = 1;
pub const MARKER_SIEGE: u8 = 2;
pub const MARKER_TOWER: u8 = 3;
pub const MARKER_BARRACKS: u8 = 4;
pub const MARKER_FORT: u8 = 5;
pub const MARKER_NEUTRAL: u8 = 6;
pub const MARKER_COURIER: u8 = 7;
pub const MARKER_RUNE: u8 = 8;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Snapshot {
    pub tick: u64,
    pub player: u32,
    pub hero: String,
    pub radiant: bool,
    pub alive: bool,
    pub respawn_in: f32,
    pub level: u32,
    pub xp: i32,
    pub xp_this_level: i32,
    pub xp_next_level: i32,
    pub gold: i32,
    pub skill_points: u32,
    pub at_shop: bool,
    pub hp: f32,
    pub max_hp: f32,
    pub hp_regen: f32,
    pub mana: f32,
    pub max_mana: f32,
    pub mana_regen: f32,
    pub strength: f32,
    pub agility: f32,
    pub intelligence: f32,
    pub armor: f32,
    pub magic_resist: f32,
    pub damage_min: f32,
    pub damage_max: f32,
    pub attack_speed: f32,
    pub move_speed: f32,
    pub abilities: Vec<AbilityView>,
    pub items: Vec<Option<ItemView>>,
    pub team: Vec<TeamHeroView>,
    pub markers: Vec<Marker>,
    pub winner: u8,
    pub stash: Vec<Option<ItemView>>,
    pub courier: u8,
    pub courier_cooldown: f32,
}

pub fn snapshot_for(m: &Match, hero: &Hero, at_shop: bool) -> Snapshot {
    let rules: &Rules = &m.rules;
    let stats = hero.stats(rules);
    let level = hero.level as usize;
    let this_level = rules.xp_to_level.get(level - 1).copied().unwrap_or(hero.xp);
    let next_level = rules.xp_to_level.get(level).copied().unwrap_or(this_level);
    let abilities = (0..ABILITY_SLOTS)
        .filter_map(|slot| {
            let name = hero.ability_name(slot)?;
            let def = m.data.ability(name)?;
            let current = hero.ability_levels[slot];
            let shown = current.max(1) as usize;
            Some(AbilityView {
                name: name.to_string(),
                level: current,
                max_level: def.max_level,
                cooldown: hero.cooldowns[slot],
                cooldown_total: def.cooldown_at(shown),
                mana: def.mana_at(shown),
                passive: def.passive(),
                can_level: hero.skill_points > 0
                    && current < def.max_level
                    && hero.level >= m.required_level(current, def.ultimate),
            })
        })
        .collect();
    let items = hero
        .items
        .iter()
        .map(|slot| {
            slot.as_ref().map(|i| ItemView {
                name: i.name.clone(),
                charges: i.charges,
                cooldown: i.cooldown,
            })
        })
        .collect();
    let team = m
        .heroes
        .iter()
        .map(|h| {
            let s = h.stats(rules);
            TeamHeroView {
                player: h.player,
                hero: h.def.name.clone(),
                radiant: h.team == Team::Radiant,
                level: h.level,
                hp_fraction: if s.max_hp > 0.0 { h.hp / s.max_hp } else { 0.0 },
                alive: h.alive,
                respawn_in: h.respawn_in,
                kills: h.kills,
                deaths: h.deaths,
                assists: h.assists,
            }
        })
        .collect();
    let mut markers: Vec<Marker> = m
        .heroes
        .iter()
        .filter(|h| h.alive)
        .map(|h| Marker {
            x: h.pos[0],
            z: h.pos[2],
            kind: MARKER_HERO,
            radiant: h.team == Team::Radiant,
            you: h.player == hero.player,
        })
        .collect();
    markers.extend(m.rune_state.runes.iter().map(|r| Marker {
        x: r.pos[0],
        z: r.pos[2],
        kind: MARKER_RUNE,
        radiant: true,
        you: false,
    }));
    markers.extend(m.units.iter().filter(|u| u.alive).map(|u| Marker {
        x: u.pos[0],
        z: u.pos[2],
        kind: match u.kind {
            crate::ai::UnitKind::Creep => MARKER_CREEP,
            crate::ai::UnitKind::Siege => MARKER_SIEGE,
            crate::ai::UnitKind::Tower => MARKER_TOWER,
            crate::ai::UnitKind::Barracks => MARKER_BARRACKS,
            crate::ai::UnitKind::Fort => MARKER_FORT,
            crate::ai::UnitKind::Neutral => MARKER_NEUTRAL,
            crate::ai::UnitKind::Courier => MARKER_COURIER,
        },
        radiant: u.team == Team::Radiant,
        you: false,
    }));
    Snapshot {
        tick: m.tick,
        player: hero.player,
        hero: hero.def.name.clone(),
        radiant: hero.team == Team::Radiant,
        alive: hero.alive,
        respawn_in: hero.respawn_in,
        level: hero.level,
        xp: hero.xp,
        xp_this_level: this_level,
        xp_next_level: next_level,
        gold: hero.gold,
        skill_points: hero.skill_points,
        at_shop,
        hp: hero.hp,
        max_hp: stats.max_hp,
        hp_regen: stats.hp_regen,
        mana: hero.mana,
        max_mana: stats.max_mana,
        mana_regen: stats.mana_regen,
        strength: stats.strength,
        agility: stats.agility,
        intelligence: stats.intelligence,
        armor: stats.armor + m.armor_bonus(hero.player),
        magic_resist: stats.magic_resist,
        damage_min: stats.damage_min,
        damage_max: stats.damage_max,
        attack_speed: stats.attack_speed,
        move_speed: stats.move_speed * m.speed_factor(hero.player as crate::combat::UnitId),
        abilities,
        items,
        team,
        markers,
        stash: hero
            .stash
            .iter()
            .map(|slot| {
                slot.as_ref().map(|i| ItemView {
                    name: i.name.clone(),
                    charges: i.charges,
                    cooldown: i.cooldown,
                })
            })
            .collect(),
        courier: match m.couriers.get(&hero.team) {
            Some(c) if c.unit.is_none() => 3,
            Some(c) => match c.task {
                crate::courier::CourierTask::Idle => 0,
                crate::courier::CourierTask::Deliver(_) => 1,
                crate::courier::CourierTask::Home => 2,
            },
            None => 3,
        },
        courier_cooldown: m.couriers.get(&hero.team).map_or(0.0, |c| c.burst_cooldown),
        winner: match m.winner {
            None => 0,
            Some(Team::Radiant) => 1,
            Some(Team::Dire) => 2,
        },
    }
}

pub const TAG_SNAPSHOT: u8 = 1;
pub const TAG_PICK: u8 = 2;
pub const TAG_SOUND: u8 = 3;

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Snapshot(Box<Snapshot>),
    Pick { taken: Vec<String> },
    Sound { event: String, pos: [f32; 3] },
}

pub fn encode_pick(taken: &[String]) -> Vec<u8> {
    let mut w = Writer::default();
    w.u8(TAG_PICK);
    let count = taken.len().min(255);
    w.u8(count as u8);
    for name in taken.iter().take(count) {
        w.str(name);
    }
    w.bytes
}

pub fn encode_sound(event: &str, pos: [f32; 3]) -> Vec<u8> {
    let mut w = Writer::default();
    w.u8(TAG_SOUND);
    w.str(event);
    w.vec3(pos);
    w.bytes
}

pub fn decode_message(bytes: &[u8]) -> Result<Message, String> {
    match bytes.first() {
        Some(&TAG_SOUND) => {
            let mut r = Reader::new(&bytes[1..]);
            let event = r.str()?;
            let pos = r.vec3()?;
            if !r.done() {
                return Err("trailing bytes".into());
            }
            Ok(Message::Sound { event, pos })
        }
        Some(&TAG_SNAPSHOT) => decode_snapshot(bytes).map(|s| Message::Snapshot(Box::new(s))),
        Some(&TAG_PICK) => {
            let mut r = Reader::new(&bytes[1..]);
            let count = r.u8()? as usize;
            let mut taken = Vec::new();
            for _ in 0..count {
                taken.push(r.str()?);
            }
            if !r.done() {
                return Err("trailing bytes".into());
            }
            Ok(Message::Pick { taken })
        }
        _ => Err("unknown message".into()),
    }
}

pub fn encode_snapshot(s: &Snapshot) -> Vec<u8> {
    let mut w = Writer::default();
    w.u8(TAG_SNAPSHOT);
    w.u32(s.tick as u32);
    w.u32(s.player);
    w.str(&s.hero);
    w.bool(s.radiant);
    w.bool(s.alive);
    w.f32(s.respawn_in);
    w.u8(s.level as u8);
    w.i32(s.xp);
    w.i32(s.xp_this_level);
    w.i32(s.xp_next_level);
    w.i32(s.gold);
    w.u8(s.skill_points as u8);
    w.bool(s.at_shop);
    for v in [
        s.hp,
        s.max_hp,
        s.hp_regen,
        s.mana,
        s.max_mana,
        s.mana_regen,
        s.strength,
        s.agility,
        s.intelligence,
        s.armor,
        s.magic_resist,
        s.damage_min,
        s.damage_max,
        s.attack_speed,
        s.move_speed,
    ] {
        w.f32(v);
    }
    w.u8(s.abilities.len() as u8);
    for a in &s.abilities {
        w.str(&a.name);
        w.u8(a.level as u8);
        w.u8(a.max_level as u8);
        w.f32(a.cooldown);
        w.f32(a.cooldown_total);
        w.f32(a.mana);
        w.bool(a.passive);
        w.bool(a.can_level);
    }
    w.u8(s.items.len() as u8);
    for item in &s.items {
        match item {
            None => w.bool(false),
            Some(i) => {
                w.bool(true);
                w.str(&i.name);
                w.i32(i.charges);
                w.f32(i.cooldown);
            }
        }
    }
    w.u8(s.team.len() as u8);
    for t in &s.team {
        w.u32(t.player);
        w.str(&t.hero);
        w.bool(t.radiant);
        w.u8(t.level as u8);
        w.f32(t.hp_fraction);
        w.bool(t.alive);
        w.f32(t.respawn_in);
        w.u16(t.kills as u16);
        w.u16(t.deaths as u16);
        w.u16(t.assists as u16);
    }
    w.u8(s.winner);
    w.u8(s.courier);
    w.f32(s.courier_cooldown);
    w.u8(s.stash.len() as u8);
    for item in &s.stash {
        match item {
            None => w.bool(false),
            Some(i) => {
                w.bool(true);
                w.str(&i.name);
                w.i32(i.charges);
                w.f32(i.cooldown);
            }
        }
    }
    let count = s.markers.len().min(400);
    w.u16(count as u16);
    for mk in s.markers.iter().take(count) {
        w.f32(mk.x);
        w.f32(mk.z);
        w.u8(mk.kind | (mk.radiant as u8) << 4 | (mk.you as u8) << 5);
    }
    w.bytes
}

pub fn decode_snapshot(bytes: &[u8]) -> Result<Snapshot, String> {
    let mut r = Reader::new(bytes);
    if r.u8()? != TAG_SNAPSHOT {
        return Err("not a snapshot".into());
    }
    let mut s = Snapshot {
        tick: r.u32()? as u64,
        player: r.u32()?,
        hero: r.str()?,
        radiant: r.bool()?,
        alive: r.bool()?,
        respawn_in: r.f32()?,
        level: r.u8()? as u32,
        xp: r.i32()?,
        xp_this_level: r.i32()?,
        xp_next_level: r.i32()?,
        gold: r.i32()?,
        skill_points: r.u8()? as u32,
        at_shop: r.bool()?,
        ..Snapshot::default()
    };
    s.hp = r.f32()?;
    s.max_hp = r.f32()?;
    s.hp_regen = r.f32()?;
    s.mana = r.f32()?;
    s.max_mana = r.f32()?;
    s.mana_regen = r.f32()?;
    s.strength = r.f32()?;
    s.agility = r.f32()?;
    s.intelligence = r.f32()?;
    s.armor = r.f32()?;
    s.magic_resist = r.f32()?;
    s.damage_min = r.f32()?;
    s.damage_max = r.f32()?;
    s.attack_speed = r.f32()?;
    s.move_speed = r.f32()?;
    let abilities = r.u8()? as usize;
    if abilities > ABILITY_SLOTS {
        return Err("too many abilities".into());
    }
    for _ in 0..abilities {
        s.abilities.push(AbilityView {
            name: r.str()?,
            level: r.u8()? as u32,
            max_level: r.u8()? as u32,
            cooldown: r.f32()?,
            cooldown_total: r.f32()?,
            mana: r.f32()?,
            passive: r.bool()?,
            can_level: r.bool()?,
        });
    }
    let items = r.u8()? as usize;
    if items > SLOTS {
        return Err("too many items".into());
    }
    for _ in 0..items {
        s.items.push(if r.bool()? {
            Some(ItemView {
                name: r.str()?,
                charges: r.i32()?,
                cooldown: r.f32()?,
            })
        } else {
            None
        });
    }
    let team = r.u8()? as usize;
    for _ in 0..team {
        s.team.push(TeamHeroView {
            player: r.u32()?,
            hero: r.str()?,
            radiant: r.bool()?,
            level: r.u8()? as u32,
            hp_fraction: r.f32()?,
            alive: r.bool()?,
            respawn_in: r.f32()?,
            kills: r.u16()? as u32,
            deaths: r.u16()? as u32,
            assists: r.u16()? as u32,
        });
    }
    s.winner = r.u8()?;
    s.courier = r.u8()?;
    s.courier_cooldown = r.f32()?;
    let stash = r.u8()? as usize;
    if stash > crate::courier::STASH_SLOTS {
        return Err("too many stash slots".into());
    }
    for _ in 0..stash {
        s.stash.push(if r.bool()? {
            Some(ItemView { name: r.str()?, charges: r.i32()?, cooldown: r.f32()? })
        } else {
            None
        });
    }
    let count = r.u16()? as usize;
    if count > 400 {
        return Err("too many markers".into());
    }
    for _ in 0..count {
        let (x, z, bits) = (r.f32()?, r.f32()?, r.u8()?);
        s.markers.push(Marker {
            x,
            z,
            kind: bits & 0x0f,
            radiant: bits & 0x10 != 0,
            you: bits & 0x20 != 0,
        });
    }
    if !r.done() {
        return Err("trailing bytes".into());
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Data, dev_data_dir};
    use std::sync::Arc;

    #[test]
    fn intents_round_trip() {
        let cases = [
            Intent::LevelUp(2),
            Intent::Cast { slot: 3, aim: Aim::Point([1.5, 2.0, -3.25]) },
            Intent::Cast { slot: 0, aim: Aim::Unit(77) },
            Intent::Cast { slot: 1, aim: Aim::Direction([0.0, 0.0, 1.0]) },
            Intent::Cast { slot: 1, aim: Aim::None },
            Intent::Buy { item: "item_tango".into(), at_shop: true },
            Intent::Sell { slot: 4, at_shop: true },
            Intent::UseItem { slot: 5, aim: Aim::None },
            Intent::SwapItems(1, 8),
            Intent::Attack { dir: [0.0, -0.5, 0.8] },
            Intent::PickHero("npc_dota_hero_axe".into()),
            Intent::Courier(crate::courier::CourierCommand::Deliver),
            Intent::Courier(crate::courier::CourierCommand::Burst),
            Intent::Courier(crate::courier::CourierCommand::Home),
        ];
        for intent in cases {
            let bytes = encode_intent(&intent);
            assert_eq!(decode_intent(&bytes, true).unwrap(), intent);
        }
    }

    #[test]
    fn the_client_cannot_say_it_is_at_the_shop() {
        let bytes = encode_intent(&Intent::Buy { item: "item_blink".into(), at_shop: true });
        match decode_intent(&bytes, false).unwrap() {
            Intent::Buy { at_shop, .. } => assert!(!at_shop),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_pick_message_and_a_snapshot_cannot_be_mistaken_for_each_other() {
        let taken = vec!["npc_dota_hero_axe".to_string(), "npc_dota_hero_sven".to_string()];
        let bytes = encode_pick(&taken);
        assert_eq!(decode_message(&bytes).unwrap(), Message::Pick { taken });
        let snap = Snapshot { tick: 0xFE, ..Snapshot::default() };
        let encoded = encode_snapshot(&snap);
        assert!(matches!(decode_message(&encoded), Ok(Message::Snapshot(_))));
        let sound = encode_sound("hero_axe.attack", [1.0, 2.0, -3.0]);
        assert_eq!(decode_message(&sound).unwrap(), Message::Sound { event: "hero_axe.attack".into(), pos: [1.0, 2.0, -3.0] });
        assert!(decode_message(&[]).is_err());
        assert!(decode_message(&[9, 9]).is_err());
        assert!(decode_snapshot(&bytes).is_err());
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(decode_intent(&[], true).is_err());
        assert!(decode_intent(&[99], true).is_err());
        assert!(decode_intent(&[1, 0, 9], true).is_err());
        assert!(decode_intent(&[0, 1, 1], true).is_err());
        let mut nan = vec![1, 0, 1];
        nan.extend_from_slice(&f32::NAN.to_le_bytes());
        nan.extend_from_slice(&[0; 8]);
        assert!(decode_intent(&nan, true).is_err());
        assert!(decode_intent(&vec![0u8; MAX_PAYLOAD + 1], true).is_err());
        assert!(decode_snapshot(&[0; 5]).is_err());
    }

    #[test]
    fn snapshot_round_trips_a_real_hero() {
        let Some(data) = dev_data_dir().and_then(|d| Data::load(&d).ok()) else { return };
        let mut m = Match::new(Arc::new(data), Rules::default());
        m.add_hero(7, Team::Dire, "npc_dota_hero_axe", [0.0; 3]).unwrap();
        m.add_hero(8, Team::Radiant, "npc_dota_hero_pudge", [0.0; 3]).unwrap();
        m.submit(7, Intent::LevelUp(0)).unwrap();
        let hero = m.hero(7).unwrap();
        let snap = snapshot_for(&m, hero, true);
        assert_eq!(snap.abilities[0].level, 1);
        assert_eq!(snap.abilities[0].name, "axe_berserkers_call");
        assert_eq!(snap.team.len(), 2);
        assert_eq!(snap.items.len(), SLOTS);
        assert_eq!(snap.xp_next_level, 240);
        assert!(!snap.radiant);
        let bytes = encode_snapshot(&snap);
        assert!(bytes.len() < MAX_PAYLOAD);
        assert_eq!(decode_snapshot(&bytes).unwrap(), snap);
    }
}
