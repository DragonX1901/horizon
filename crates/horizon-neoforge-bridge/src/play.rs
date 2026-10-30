//! Play state, 767 ↔ 777. A packet whose wire format is the same in both is
//! passed on under its new id (matched by name); the ones that changed are
//! rebuilt; the rest are dropped until they get a translator.
use crate::blocks::StateMap;
use crate::ids_767 as v767;
use horizon_protocol::buf::{Codec, Reader, Writer};
use horizon_protocol::chunk::{
    BlockEntityInfo, ChunkData, LightData, Paletted, Section, Strategy, storage_get,
};
use horizon_protocol::ids as v777;
use horizon_protocol::packets::play::{self as p777, Clientbound as C777, Serverbound as S777};
use horizon_protocol::registry::ceil_log2;
use horizon_protocol::{Error, Result};
use horizon_world::nbt::Tag;
use std::collections::HashMap;

/// Clientbound packets whose 767 and 777 bodies are the same.
const SAME_CLIENTBOUND: &[&str] = &[
    "keep_alive",
    "ping",
    "set_chunk_cache_center",
    "set_chunk_cache_radius",
    "chunk_batch_start",
    "chunk_batch_finished",
    "forget_level_chunk",
    "light_update",
    "game_event",
    "set_health",
    "set_experience",
    "player_abilities",
    "change_difficulty",
    "disconnect",
    "system_chat",
    "set_action_bar_text",
    "set_title_text",
    "set_subtitle_text",
    "set_titles_animation",
    "clear_titles",
    "tab_list",
    "block_destruction",
    "block_changed_ack",
    "initialize_border",
    "set_border_center",
    "set_border_lerp_size",
    "set_border_size",
    "set_border_warning_delay",
    "set_border_warning_distance",
    "ticking_state",
    "ticking_step",
];
/// Serverbound packets whose 777 and 767 bodies are the same (the move packets'
/// flags byte reads as 767's on-ground boolean: any non-zero byte is true).
const SAME_SERVERBOUND: &[&str] = &[
    "keep_alive",
    "pong",
    "chunk_batch_received",
    "move_player_pos",
    "move_player_pos_rot",
    "move_player_rot",
    "move_player_status_only",
    "player_action",
    "use_item",
    "set_carried_item",
    "chat_command",
    "container_close",
];

/// Name → id over a generated `NAMES` table.
fn index(names: &'static [&'static str]) -> HashMap<&'static str, i32> {
    names
        .iter()
        .enumerate()
        .map(|(i, n)| (*n, i as i32))
        .collect()
}

/// Server facts the translation needs, from configuration.
pub struct Context {
    pub states: StateMap,
    /// The client's global palette width (its whole block-state registry).
    pub client_block_bits: u8,
    /// Per client state: whether it holds a fluid (LevelChunkSection.fluidCount).
    pub client_fluid: Vec<bool>,
    /// The server's global palette width (1.21.1 plus mods).
    pub server_block_bits: u8,
    /// The biome registry's size (sent to the client as the server ordered it).
    pub biomes: usize,
    /// Dimension types in registry order: (name, min_y, height).
    pub dimension_types: Vec<(String, i32, i32)>,
    /// Block entity types: server id → client id.
    pub block_entity_types: HashMap<i32, i32>,
}

pub struct Play {
    ctx: Context,
    clientbound: HashMap<i32, i32>,
    serverbound: HashMap<i32, i32>,
    /// The player's entity id (for player commands) and current level.
    player: i32,
    dimension: String,
    sections: usize,
    /// The 777 input flags last sent, to derive 767's sneak and sprint commands.
    input: u8,
}

impl Play {
    pub fn new(ctx: Context) -> Self {
        let c767 = index(v767::play::clientbound::NAMES);
        let c777 = index(v777::play::clientbound::NAMES);
        let s767 = index(v767::play::serverbound::NAMES);
        let s777 = index(v777::play::serverbound::NAMES);
        let clientbound = SAME_CLIENTBOUND
            .iter()
            .filter_map(|n| Some((*c767.get(n)?, *c777.get(n)?)))
            .collect();
        let serverbound = SAME_SERVERBOUND
            .iter()
            .filter_map(|n| Some((*s777.get(n)?, *s767.get(n)?)))
            .collect();
        Self {
            ctx,
            clientbound,
            serverbound,
            player: 0,
            dimension: "minecraft:overworld".into(),
            sections: 24,
            input: 0,
        }
    }

    fn enter(&mut self, dimension_type: i32, dimension: String) {
        let height = usize::try_from(dimension_type)
            .ok()
            .and_then(|i| self.ctx.dimension_types.get(i))
            .map_or(384, |d| d.2);
        self.sections = (height / 16) as usize;
        self.dimension = dimension;
    }

    /// A 767 clientbound packet as the 777 packets to send for it.
    pub fn clientbound(&mut self, id: i32, body: &[u8]) -> Result<Vec<Vec<u8>>> {
        use v767::play::clientbound as cb;
        if let Some(&to) = self.clientbound.get(&id) {
            let mut out = Writer::new();
            out.varint(to);
            out.bytes(body);
            return Ok(vec![out.data]);
        }
        let mut r = Reader::new(body);
        let packet = match id {
            cb::LOGIN => {
                let player_id = r.i32()?;
                let hardcore = r.bool()?;
                let levels = (0..r.count(1 << 12)?)
                    .map(|_| r.string(32767))
                    .collect::<Result<_>>()?;
                let max_players = r.varint()?;
                let chunk_radius = r.varint()?;
                let simulation_distance = r.varint()?;
                let reduced_debug_info = r.bool()?;
                let show_death_screen = r.bool()?;
                let limited_crafting = r.bool()?;
                let spawn = spawn_info(&mut r)?;
                let enforces_secure_chat = r.bool()?;
                self.player = player_id;
                self.enter(spawn.dimension_type, spawn.dimension.clone());
                C777::Login(p777::Login {
                    player_id,
                    hardcore,
                    levels,
                    max_players,
                    chunk_radius,
                    simulation_distance,
                    reduced_debug_info,
                    show_death_screen,
                    limited_crafting,
                    spawn,
                    online_mode: false,
                    enforces_secure_chat,
                })
            }
            cb::RESPAWN => {
                let spawn = spawn_info(&mut r)?;
                let keep = r.i8()?;
                self.enter(spawn.dimension_type, spawn.dimension.clone());
                C777::Respawn(p777::Respawn { spawn, keep })
            }
            cb::LEVEL_CHUNK_WITH_LIGHT => C777::LevelChunkWithLight(self.chunk(&mut r)?),
            cb::BLOCK_UPDATE => {
                let pos = unpack_pos(r.i64()?);
                let state = self.ctx.states.map(r.varint()?);
                C777::BlockUpdate(p777::BlockUpdate { pos, state })
            }
            cb::SECTION_BLOCKS_UPDATE => {
                let section = r.i64()?;
                let changes = (0..r.count(4096)?)
                    .map(|_| {
                        let c = r.varlong()?;
                        let state = self.ctx.states.map((c >> 12) as i32);
                        Ok(i64::from(state) << 12 | (c & 0xFFF))
                    })
                    .collect::<Result<_>>()?;
                C777::SectionBlocksUpdate(p777::SectionBlocksUpdate {
                    section: unpack_section(section),
                    changes,
                })
            }
            cb::PLAYER_POSITION => {
                let position = [r.f64()?, r.f64()?, r.f64()?];
                let (yaw, pitch) = (r.f32()?, r.f32()?);
                // 767's relative bits are 777's first five.
                let relatives = i32::from(r.u8()?) & 0x1F;
                let teleport = r.varint()?;
                C777::PlayerPosition(p777::PlayerPosition {
                    teleport,
                    position,
                    delta: [0.0; 3],
                    yaw,
                    pitch,
                    relatives,
                })
            }
            cb::SET_DEFAULT_SPAWN_POSITION => {
                let pos = unpack_pos(r.i64()?);
                let yaw = r.f32()?;
                C777::SetDefaultSpawnPosition(p777::SetDefaultSpawnPosition {
                    pos: p777::GlobalPos {
                        dimension: self.dimension.clone(),
                        pos,
                    },
                    yaw,
                    pitch: 0.0,
                })
            }
            cb::SET_TIME => {
                let game_time = r.i64()?;
                let day_time = r.i64()?;
                // A negative day time means the daylight cycle is off.
                C777::SetTime(p777::SetTime {
                    game_time,
                    clocks: vec![p777::ClockState {
                        clock: 0,
                        total_ticks: day_time.abs(),
                        partial_tick: 0.0,
                        rate: if day_time < 0 { 0.0 } else { 1.0 },
                    }],
                })
            }
            _ => return Ok(Vec::new()),
        };
        Ok(vec![packet.encode()])
    }

    /// LevelChunkWithLight: sections re-encoded with the client's block ids,
    /// heightmaps from NBT to their 777 list, block entity types remapped.
    fn chunk(&self, r: &mut Reader) -> Result<p777::LevelChunkWithLight> {
        let x = r.i32()?;
        let z = r.i32()?;
        let heightmaps = heightmaps(&r.nbt()?);
        let buffer = r.byte_array(r.remaining())?;
        let mut block_entities = Vec::new();
        for _ in 0..r.count(1 << 16)? {
            let packed_xz = r.i8()?;
            let y = r.i16()?;
            let kind = r.varint()?;
            let tag = r.nbt()?;
            if let Some(&kind) = self.ctx.block_entity_types.get(&kind) {
                block_entities.push(BlockEntityInfo {
                    packed_xz,
                    y,
                    kind,
                    tag,
                });
            }
        }
        let light = LightData::read(r)?;
        let mut sections = Vec::with_capacity(self.sections);
        let mut b = Reader::new(&buffer);
        for _ in 0..self.sections {
            sections.push(self.section(&mut b)?);
        }
        Ok(p777::LevelChunkWithLight {
            x,
            z,
            chunk: ChunkData {
                heightmaps,
                buffer: ChunkData::from_sections(&sections),
                block_entities,
            },
            light,
        })
    }

    fn section(&self, r: &mut Reader) -> Result<Section> {
        let non_empty = r.i16()?;
        let states = read_767(r, Strategy::Blocks, self.ctx.server_block_bits)?;
        let biomes = read_767(r, Strategy::Biomes, ceil_log2(self.ctx.biomes))?;
        let states: Vec<i32> = states.into_iter().map(|s| self.ctx.states.map(s)).collect();
        let fluids = states
            .iter()
            .filter(|&&s| {
                self.ctx
                    .client_fluid
                    .get(s as usize)
                    .copied()
                    .unwrap_or(false)
            })
            .count() as i16;
        Ok(Section {
            non_empty,
            fluids,
            states: Paletted::from_values(&states, Strategy::Blocks, self.ctx.client_block_bits),
            biomes: Paletted::from_values(&biomes, Strategy::Biomes, ceil_log2(self.ctx.biomes)),
        })
    }

    /// A 777 serverbound packet as the 767 packets to send for it.
    pub fn serverbound(&mut self, id: i32, body: &[u8]) -> Result<Vec<Vec<u8>>> {
        use v767::play::serverbound as sb;
        if let Some(&to) = self.serverbound.get(&id) {
            let mut out = Writer::new();
            out.varint(to);
            out.bytes(body);
            return Ok(vec![out.data]);
        }
        let packet = |id: i32, f: &dyn Fn(&mut Writer)| {
            let mut w = Writer::new();
            w.varint(id);
            f(&mut w);
            w.data
        };
        let Ok(decoded) = S777::decode(id, body) else {
            return Ok(Vec::new());
        };
        let player = self.player;
        Ok(match decoded {
            S777::AcceptTeleportation(t) => {
                vec![packet(sb::ACCEPT_TELEPORTATION, &|w| w.varint(t.id))]
            }
            S777::UseItemOn(u) => vec![packet(sb::USE_ITEM_ON, &|w| {
                w.varint(u.hand);
                w.i64(pack_pos(u.pos));
                w.varint(u.face);
                for c in u.cursor {
                    w.f32(c);
                }
                w.bool(u.inside);
                w.varint(u.sequence);
            })],
            S777::Punch(_) => vec![packet(sb::SWING, &|w| w.varint(0))],
            S777::Attack(a) => vec![packet(sb::INTERACT, &|w| {
                w.varint(a.id);
                w.varint(1);
                w.bool(self.input & 32 != 0);
            })],
            S777::Interact(i) => {
                let sneaking = self.input & 32 != 0;
                vec![
                    packet(sb::INTERACT, &|w| {
                        w.varint(i.id);
                        w.varint(2);
                        for c in i.location {
                            w.f32(c as f32);
                        }
                        w.varint(i.hand);
                        w.bool(sneaking);
                    }),
                    packet(sb::INTERACT, &|w| {
                        w.varint(i.id);
                        w.varint(0);
                        w.varint(i.hand);
                        w.bool(sneaking);
                    }),
                ]
            }
            S777::PlayerCommand(c) => {
                // 777 dropped 767's sneak actions (0, 1): every later one moved up by two.
                vec![packet(sb::PLAYER_COMMAND, &|w| {
                    w.varint(c.id);
                    w.varint(c.action + 2);
                    w.varint(c.data);
                })]
            }
            S777::PlayerInput(i) => {
                let flags = i.flags;
                let axis = |pos: u8, neg: u8| {
                    (if flags & pos != 0 { 1.0f32 } else { 0.0 })
                        - if flags & neg != 0 { 1.0 } else { 0.0 }
                };
                let mut out = vec![packet(sb::PLAYER_INPUT, &|w| {
                    w.f32(axis(4, 8));
                    w.f32(axis(1, 2));
                    w.u8(u8::from(flags & 16 != 0) | u8::from(flags & 32 != 0) << 1);
                })];
                // Sneaking was a player command in 767.
                if (flags ^ self.input) & 32 != 0 {
                    let action = if flags & 32 != 0 { 0 } else { 1 };
                    out.push(packet(sb::PLAYER_COMMAND, &|w| {
                        w.varint(player);
                        w.varint(action);
                        w.varint(0);
                    }));
                }
                self.input = flags;
                out
            }
            S777::ClientCommand(c) if c.action <= 1 => {
                vec![packet(sb::CLIENT_COMMAND, &|w| w.varint(c.action))]
            }
            _ => Vec::new(),
        })
    }
}

/// CommonPlayerSpawnInfo, 767: the game type is a byte and the previous one a byte
/// (-1: none); there is no sea level.
fn spawn_info(r: &mut Reader) -> Result<p777::SpawnInfo> {
    let dimension_type = r.varint()?;
    let dimension = r.string(32767)?;
    let seed = r.i64()?;
    let game_type = i32::from(r.u8()?);
    let previous = r.i8()?;
    let debug = r.bool()?;
    let flat = r.bool()?;
    let last_death = if r.bool()? {
        Some(p777::GlobalPos {
            dimension: r.string(32767)?,
            pos: unpack_pos(r.i64()?),
        })
    } else {
        None
    };
    let portal_cooldown = r.varint()?;
    Ok(p777::SpawnInfo {
        dimension_type,
        dimension,
        seed,
        game_type,
        previous_game_type: (previous >= 0).then_some(i32::from(previous)),
        debug,
        flat,
        last_death,
        portal_cooldown,
        sea_level: 63,
    })
}

/// A 767 PalettedContainer's values: like 777's, but its data array is
/// length-prefixed.
fn read_767(r: &mut Reader, strategy: Strategy, global_bits: u8) -> Result<Vec<i32>> {
    let entries = match strategy {
        Strategy::Blocks => 4096,
        Strategy::Biomes => 64,
    };
    let wire = r.u8()?;
    let (palette, bits) = match (strategy, wire) {
        (_, 0) => (true, 0),
        (Strategy::Blocks, 1..=4) => (true, 4),
        (Strategy::Blocks, 5..=8) => (true, wire),
        (Strategy::Biomes, 1..=3) => (true, wire),
        _ => (false, global_bits.max(wire)),
    };
    if bits > 32 {
        return Err(Error::new(format!("{bits} bits per entry")));
    }
    let palette: Vec<i32> = if !palette {
        Vec::new()
    } else if bits == 0 {
        vec![r.varint()?]
    } else {
        (0..r.count(1 << bits)?)
            .map(|_| r.varint())
            .collect::<Result<_>>()?
    };
    let n = r.count(1 << 16)?;
    let data = r.fixed_longs(n)?;
    Ok((0..entries)
        .map(|i| {
            if bits == 0 {
                return palette.first().copied().unwrap_or(0);
            }
            let v = storage_get(&data, bits, i);
            if palette.is_empty() {
                v as i32
            } else {
                palette.get(v as usize).copied().unwrap_or(0)
            }
        })
        .collect())
}

/// 767 heightmaps (an NBT compound of long arrays by type name) as 777's (type id, longs).
fn heightmaps(tag: &Tag) -> Vec<(i32, Vec<i64>)> {
    const TYPES: [&str; 6] = [
        "WORLD_SURFACE_WG",
        "WORLD_SURFACE",
        "OCEAN_FLOOR_WG",
        "OCEAN_FLOOR",
        "MOTION_BLOCKING",
        "MOTION_BLOCKING_NO_LEAVES",
    ];
    TYPES
        .iter()
        .enumerate()
        .filter_map(|(i, name)| match tag.get(name) {
            Some(Tag::LongArray(longs)) => Some((i as i32, longs.clone())),
            _ => None,
        })
        .collect()
}

/// BlockPos.asLong: x 26 bits, z 26 bits, y 12 bits.
fn unpack_pos(v: i64) -> [i32; 3] {
    [
        (v >> 38) as i32,
        (v << 52 >> 52) as i32,
        (v << 26 >> 38) as i32,
    ]
}
fn pack_pos(p: [i32; 3]) -> i64 {
    (i64::from(p[0]) & 0x3FF_FFFF) << 38
        | (i64::from(p[2]) & 0x3FF_FFFF) << 12
        | (i64::from(p[1]) & 0xFFF)
}
/// SectionPos.asLong: x 22 bits, z 22 bits, y 20 bits.
fn unpack_section(v: i64) -> [i32; 3] {
    [
        (v >> 42) as i32,
        (v << 44 >> 44) as i32,
        (v << 22 >> 42) as i32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_round_trip() {
        for p in [
            [0, 0, 0],
            [-1, -64, -1],
            [30_000_000, 319, -30_000_000],
            [5, -1, 7],
        ] {
            assert_eq!(unpack_pos(pack_pos(p)), p);
        }
        let s = (5i64 & 0x3F_FFFF) << 42 | (-3i64 & 0x3F_FFFF) << 20 | (-4i64 & 0xF_FFFF);
        assert_eq!(unpack_section(s), [5, -4, -3]);
    }

    #[test]
    fn container_767() {
        // Single value; then an indirect 4-bit palette of two values.
        let mut w = Writer::new();
        w.u8(0);
        w.varint(7);
        w.varint(0);
        assert_eq!(
            read_767(&mut Reader::new(&w.data), Strategy::Biomes, 6).unwrap(),
            vec![7; 64]
        );
        let mut w = Writer::new();
        w.u8(4);
        w.varint(2);
        w.varint(10);
        w.varint(20);
        w.varint(256);
        for _ in 0..256 {
            w.i64(0x1111_1111_1111_1111);
        }
        assert_eq!(
            read_767(&mut Reader::new(&w.data), Strategy::Blocks, 15).unwrap(),
            vec![20; 4096]
        );
    }
}
