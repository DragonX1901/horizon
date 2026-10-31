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
    "move_entity_rot",
    "rotate_head",
    "remove_entities",
    "entity_event",
    "animate",
    "take_item_entity",
    "hurt_animation",
    "set_entity_link",
    "set_passengers",
    "player_info_update",
    "player_info_remove",
    "boss_event",
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
    "select_trade",
    "rename_item",
    // Mods' own payloads (Simple Voice Chat's) pass through as they are.
    "custom_payload",
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
    /// Items, entity types and menus: server id → client id.
    pub items: HashMap<i32, i32>,
    pub entity_types: HashMap<i32, i32>,
    pub menus: HashMap<i32, i32>,
    /// Sound events: server id → the client's id, or the name to send inline.
    pub sounds: HashMap<i32, std::result::Result<i32, String>>,
    /// Blocks (the registry, not states): server id → client id.
    pub blocks: HashMap<i32, i32>,
    /// Mob effects and attributes: server id → client id.
    pub effects: HashMap<i32, i32>,
    pub attributes: HashMap<i32, i32>,
    /// Particle types: server id → (client id, name).
    pub particles: HashMap<i32, (i32, String)>,
    /// Command argument types: server id → client id; and the client's
    /// `brigadier:string`, which stands in for parsers it lacks.
    pub arguments: HashMap<i32, i32>,
    pub string_argument: Option<i32>,
    /// The client's `minecraft:generic_9x1` … `generic_9x6` menus, for mod menus.
    pub generic_menus: [Option<i32>; 6],
    /// Data component types: server id → client id.
    pub components: HashMap<i32, i32>,
    /// The client's item for server items it lacks.
    pub placeholder_item: i32,
    /// The mod entity types given client ids past its registry, to announce.
    pub modded_entities: Vec<horizon_protocol::modded::ModdedEntity>,
    /// The client's `minecraft:experience_orb` entity type.
    pub experience_orb: Option<i32>,
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
    /// Items: client id → server id, for stacks the client sends.
    items_back: HashMap<i32, i32>,
    /// Mod menus the client lacks: waiting for their contents (title), and
    /// shown as chests (by container id).
    pending_screens: HashMap<i32, horizon_world::nbt::Tag>,
    /// NeoForge's split packets in play.
    splitter: crate::neoforge::Splitter,
    generic: HashMap<i32, Generic>,
}

/// A mod's menu shown as a chest: its `mod_slots` fill the first of `rows`
/// rows of nine, the rest padding, then the player's 36 slots.
struct Generic {
    mod_slots: usize,
    rows: usize,
}
impl Generic {
    fn shown(&self) -> usize {
        self.mod_slots.min(self.rows * 9)
    }
    /// The server's slot list as the chest's: mod slots, padding, the player's.
    fn to_client(
        &self,
        mut items: Vec<Option<horizon_protocol::item::Stack>>,
    ) -> Vec<Option<horizon_protocol::item::Stack>> {
        let player = items.split_off(self.mod_slots.min(items.len()));
        items.truncate(self.shown());
        items.resize(self.rows * 9, None);
        items.extend(player);
        items
    }
    fn client_slot(&self, server: i16) -> Option<i16> {
        let s = usize::try_from(server).ok()?;
        if s < self.mod_slots {
            (s < self.shown()).then_some(server)
        } else {
            Some((s - self.mod_slots + self.rows * 9) as i16)
        }
    }
    fn server_slot(&self, client: i16) -> Option<i16> {
        // -999: a click outside the window (drop the carried stack).
        let Ok(c) = usize::try_from(client) else {
            return Some(client);
        };
        if c < self.shown() {
            Some(client)
        } else if c < self.rows * 9 {
            None
        } else {
            Some((c - self.rows * 9 + self.mod_slots) as i16)
        }
    }
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
        let items_back = ctx
            .items
            .iter()
            .map(|(&server, &client)| (client, server))
            .collect();
        Self {
            pending_screens: HashMap::new(),
            splitter: Default::default(),
            generic: HashMap::new(),
            items_back,
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
            cb::LIGHT_UPDATE => C777::LightUpdate(p777::LightUpdate {
                x: r.varint()?,
                z: r.varint()?,
                light: light_767(&mut r)?,
            }),
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
            cb::ADD_ENTITY => {
                let id = r.varint()?;
                let uuid = r.uuid()?;
                let Some(&kind) = self.ctx.entity_types.get(&r.varint()?) else {
                    return Ok(Vec::new());
                };
                let position = [r.f64()?, r.f64()?, r.f64()?];
                let (pitch, yaw, head_yaw) = (r.i8()?, r.i8()?, r.i8()?);
                let data = r.varint()?;
                let velocity = [velocity(r.i16()?), velocity(r.i16()?), velocity(r.i16()?)];
                C777::AddEntity(p777::AddEntity {
                    id,
                    uuid,
                    kind,
                    position,
                    velocity,
                    pitch,
                    yaw,
                    head_yaw,
                    data,
                })
            }
            cb::ADD_EXPERIENCE_ORB => {
                let Some(kind) = self.ctx.experience_orb else {
                    return Ok(Vec::new());
                };
                let id = r.varint()?;
                let position = [r.f64()?, r.f64()?, r.f64()?];
                let value = i32::from(r.i16()?);
                // 777 spawns orbs as ordinary entities, their value in the data field.
                C777::AddEntity(p777::AddEntity {
                    id,
                    uuid: u128::from(id as u32),
                    kind,
                    position,
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: value,
                })
            }
            // 767: id, deltas, [rotation], on ground. 777 puts on-ground (bit 0) and
            // a step count (0: one linear move) in a varint ahead of the deltas.
            cb::MOVE_ENTITY_POS | cb::MOVE_ENTITY_POS_ROT => {
                let rot = id == cb::MOVE_ENTITY_POS_ROT;
                let entity = r.varint()?;
                let delta = [r.i16()?, r.i16()?, r.i16()?];
                let rotation = if rot { Some((r.i8()?, r.i8()?)) } else { None };
                let on_ground = r.bool()?;
                let mut w = Writer::new();
                w.varint(if rot {
                    v777::play::clientbound::MOVE_ENTITY_POS_ROT
                } else {
                    v777::play::clientbound::MOVE_ENTITY_POS
                });
                w.varint(entity);
                w.varint(i32::from(on_ground));
                for d in delta {
                    w.i16(d);
                }
                if let Some((yaw, pitch)) = rotation {
                    w.i8(yaw);
                    w.i8(pitch);
                }
                return Ok(vec![w.data]);
            }
            cb::SET_ENTITY_MOTION => {
                let id = r.varint()?;
                let velocity = [velocity(r.i16()?), velocity(r.i16()?), velocity(r.i16()?)];
                C777::SetEntityMotion(p777::SetEntityMotion { id, velocity })
            }
            cb::TELEPORT_ENTITY => {
                let id = r.varint()?;
                let position = [r.f64()?, r.f64()?, r.f64()?];
                let yaw = f32::from(r.i8()?) * 360.0 / 256.0;
                let pitch = f32::from(r.i8()?) * 360.0 / 256.0;
                let on_ground = r.bool()?;
                C777::TeleportEntity(p777::TeleportEntity {
                    id,
                    position,
                    delta: [0.0; 3],
                    yaw,
                    pitch,
                    relatives: 0,
                    on_ground,
                })
            }
            cb::SET_ENTITY_DATA => {
                let mut w = Writer::new();
                w.varint(v777::play::clientbound::SET_ENTITY_DATA);
                w.varint(r.varint()?);
                self.entity_data(&mut r, &mut w)?;
                return Ok(vec![w.data]);
            }
            cb::SET_EQUIPMENT => {
                let id = r.varint()?;
                let mut slots = Vec::new();
                loop {
                    let slot = r.u8()?;
                    slots.push((slot & 127, self.stack(&mut r)?));
                    if slot & 128 == 0 {
                        break;
                    }
                }
                C777::SetEquipment(p777::SetEquipment { id, slots })
            }
            cb::CONTAINER_SET_CONTENT => {
                let container = i32::from(r.u8()?);
                let state = r.varint()?;
                let mut items: Vec<_> = (0..r.count(1 << 12)?)
                    .map(|_| self.stack(&mut r))
                    .collect::<Result<_>>()?;
                let carried = self.stack(&mut r)?;
                let mut out = Vec::new();
                // A mod's menu the client lacks opens as a chest of enough rows.
                if let Some(title) = self.pending_screens.remove(&container) {
                    let mod_slots = items.len().saturating_sub(36);
                    let rows = mod_slots.div_ceil(9).clamp(1, 6);
                    if let Some(menu) = self.ctx.generic_menus[rows - 1] {
                        self.generic.insert(container, Generic { mod_slots, rows });
                        out.push(
                            C777::OpenScreen(p777::OpenScreen {
                                container,
                                menu,
                                title,
                            })
                            .encode(),
                        );
                    }
                }
                if let Some(g) = self.generic.get(&container) {
                    items = g.to_client(items);
                }
                out.push(
                    C777::ContainerSetContent(p777::ContainerSetContent {
                        container,
                        state,
                        items,
                        carried,
                    })
                    .encode(),
                );
                return Ok(out);
            }
            cb::CONTAINER_SET_SLOT => {
                let container = i32::from(r.i8()?);
                let state = r.varint()?;
                let mut slot = r.i16()?;
                if let Some(g) = self.generic.get(&container) {
                    match g.client_slot(slot) {
                        Some(s) => slot = s,
                        None => return Ok(Vec::new()),
                    }
                }
                let item = self.stack(&mut r)?;
                C777::ContainerSetSlot(p777::ContainerSetSlot {
                    container,
                    state,
                    slot,
                    item,
                })
            }
            cb::CONTAINER_SET_DATA => {
                let container = i32::from(r.u8()?);
                C777::ContainerSetData(p777::ContainerSetData {
                    container,
                    id: r.i16()?,
                    value: r.i16()?,
                })
            }
            cb::CONTAINER_CLOSE => {
                let container = i32::from(r.u8()?);
                self.generic.remove(&container);
                C777::ContainerClose(p777::ContainerClose { container })
            }
            cb::OPEN_SCREEN => {
                let container = r.varint()?;
                let menu = r.varint()?;
                return Ok(self.open_screen(container, menu, r.nbt()?));
            }
            cb::CUSTOM_PAYLOAD => {
                let channel = r.string(32767)?;
                match channel.as_str() {
                    // NeoForge's open screen for menus with extra data (most mods' machines).
                    crate::neoforge::ADVANCED_OPEN_SCREEN => {
                        let container = r.varint()?;
                        let menu = r.varint()?;
                        return Ok(self.open_screen(container, menu, r.nbt()?));
                    }
                    crate::neoforge::SPLIT => {
                        if let Some(whole) = self.splitter.accept(r.rest())? {
                            let mut w = Reader::new(&whole);
                            let id = w.varint()?;
                            return self.clientbound(id, w.rest());
                        }
                    }
                    channel
                        if channel.starts_with("neoforge:")
                            || channel.starts_with("minecraft:") => {}
                    // Mods' own payloads go on to the client, which reads those it knows
                    // (Simple Voice Chat's) and ignores the rest.
                    channel => {
                        let mut w = Writer::new();
                        w.varint(horizon_protocol::ids::play::clientbound::CUSTOM_PAYLOAD);
                        w.string(channel);
                        w.bytes(r.rest());
                        return Ok(vec![w.data]);
                    }
                }
                return Ok(Vec::new());
            }
            cb::SET_CARRIED_ITEM => C777::SetHeldSlot(p777::SetHeldSlot {
                slot: i32::from(r.i8()?),
            }),
            cb::SOUND | cb::SOUND_ENTITY => {
                let Some(sound) = self.sound(&mut r)? else {
                    return Ok(Vec::new());
                };
                let source = r.varint()?;
                if id == cb::SOUND {
                    C777::Sound(p777::Sound {
                        sound,
                        source,
                        x: r.i32()?,
                        y: r.i32()?,
                        z: r.i32()?,
                        volume: r.f32()?,
                        pitch: r.f32()?,
                        seed: r.i64()?,
                    })
                } else {
                    C777::SoundEntity(p777::SoundEntity {
                        sound,
                        source,
                        id: r.varint()?,
                        volume: r.f32()?,
                        pitch: r.f32()?,
                        seed: r.i64()?,
                    })
                }
            }
            cb::LEVEL_EVENT => {
                let kind = r.i32()?;
                let pos = unpack_pos(r.i64()?);
                let data = r.i32()?;
                // 2001 (block destroyed) carries the block's state.
                let data = if kind == 2001 {
                    self.ctx.states.map(data)
                } else {
                    data
                };
                C777::LevelEvent(p777::LevelEvent {
                    kind,
                    pos,
                    data,
                    global: r.bool()?,
                })
            }
            cb::BLOCK_EVENT => {
                let pos = unpack_pos(r.i64()?);
                let (b0, b1) = (r.u8()?, r.u8()?);
                let Some(&block) = self.ctx.blocks.get(&r.varint()?) else {
                    return Ok(Vec::new());
                };
                C777::BlockEvent(p777::BlockEvent { pos, b0, b1, block })
            }
            cb::BLOCK_ENTITY_DATA => {
                let pos = unpack_pos(r.i64()?);
                let Some(&kind) = self.ctx.block_entity_types.get(&r.varint()?) else {
                    return Ok(Vec::new());
                };
                C777::BlockEntityData(p777::BlockEntityData {
                    pos,
                    kind,
                    tag: r.nbt()?,
                })
            }
            cb::UPDATE_MOB_EFFECT => {
                let id = r.varint()?;
                let Some(&effect) = self.ctx.effects.get(&r.varint()?) else {
                    return Ok(Vec::new());
                };
                C777::UpdateMobEffect(p777::UpdateMobEffect {
                    id,
                    effect,
                    amplifier: r.varint()?,
                    duration: r.varint()?,
                    flags: r.i8()?,
                })
            }
            cb::REMOVE_MOB_EFFECT => {
                let id = r.varint()?;
                let Some(&effect) = self.ctx.effects.get(&r.varint()?) else {
                    return Ok(Vec::new());
                };
                C777::RemoveMobEffect(p777::RemoveMobEffect { id, effect })
            }
            cb::UPDATE_ATTRIBUTES => {
                let id = r.varint()?;
                let mut values = Vec::new();
                for _ in 0..r.count(1 << 10)? {
                    let attribute = r.varint()?;
                    let base = r.f64()?;
                    let modifiers = (0..r.count(1 << 10)?)
                        .map(|_| {
                            Ok(p777::AttributeModifier {
                                id: r.string(32767)?,
                                amount: r.f64()?,
                                operation: i32::from(r.u8()?),
                            })
                        })
                        .collect::<Result<_>>()?;
                    if let Some(&attribute) = self.ctx.attributes.get(&attribute) {
                        values.push(p777::AttributeSnapshot {
                            attribute,
                            base,
                            modifiers,
                        });
                    }
                }
                C777::UpdateAttributes(p777::UpdateAttributes { id, values })
            }
            cb::LEVEL_PARTICLES => {
                let override_limiter = r.bool()?;
                let pos = [r.f64()?, r.f64()?, r.f64()?];
                let dist = [r.f32()?, r.f32()?, r.f32()?];
                let speed = r.f32()?;
                let count = r.i32()?;
                let Some(particle) = self.particle(&mut r)? else {
                    return Ok(Vec::new());
                };
                C777::LevelParticles(p777::LevelParticles {
                    particle,
                    override_limiter,
                    always_show: false,
                    pos,
                    dist,
                    max_speed: [speed; 3],
                    count,
                    randomization: 0,
                })
            }
            cb::COMMANDS => C777::Commands(self.commands(&mut r)?),
            // 767 writes the parameters as display name, options, the name tag
            // and collision rules by name, the colour as a ChatFormatting ordinal,
            // prefix, suffix; 777 as display name, prefix, suffix, the two rules as
            // enum ids, an optional colour, options.
            cb::SET_PLAYER_TEAM => {
                let name = r.string(32767)?;
                let method = r.i8()?;
                let parameters = if method == 0 || method == 2 {
                    let display_name = r.nbt()?;
                    let options = r.i8()?;
                    let rule = |s: &str, names: [&str; 4]| {
                        names.iter().position(|n| *n == s).unwrap_or(0) as i32
                    };
                    let name_tag_visibility = rule(
                        &r.string(40)?,
                        ["always", "never", "hideForOtherTeams", "hideForOwnTeam"],
                    );
                    let collision_rule = rule(
                        &r.string(40)?,
                        ["always", "never", "pushOtherTeams", "pushOwnTeam"],
                    );
                    let color = r.varint()?;
                    Some(p777::TeamParameters {
                        display_name,
                        prefix: r.nbt()?,
                        suffix: r.nbt()?,
                        name_tag_visibility,
                        collision_rule,
                        color: (0..16).contains(&color).then_some(color),
                        options,
                    })
                } else {
                    None
                };
                let players = if matches!(method, 0 | 3 | 4) {
                    (0..r.count(1 << 16)?)
                        .map(|_| r.string(32767))
                        .collect::<Result<_>>()?
                } else {
                    Vec::new()
                };
                C777::SetPlayerTeam(p777::SetPlayerTeam {
                    name,
                    method,
                    parameters,
                    players,
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
        let light = light_767(r)?;
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

    /// A 767 particle (type, then its options), when the client has the type and
    /// its options are ones 777 still encodes the same or that translate.
    fn particle(&self, r: &mut Reader) -> Result<Option<horizon_protocol::entity_data::Particle>> {
        use horizon_protocol::entity_data::{Particle, ParticleData};
        let Some((id, name)) = self.ctx.particles.get(&r.varint()?) else {
            return Ok(None);
        };
        let rgb = |r: &mut Reader| -> Result<i32> {
            let c = |v: f32| ((v.clamp(0.0, 1.0) * 255.0).round() as i32) & 0xFF;
            Ok(0xFF << 24 | c(r.f32()?) << 16 | c(r.f32()?) << 8 | c(r.f32()?))
        };
        let data = match name.as_str() {
            "minecraft:block"
            | "minecraft:block_marker"
            | "minecraft:falling_dust"
            | "minecraft:dust_pillar" => {
                Some(ParticleData::State(self.ctx.states.map(r.varint()?)))
            }
            "minecraft:dust" => {
                let color = rgb(r)?;
                Some(ParticleData::ColorScale(color, r.f32()?))
            }
            "minecraft:dust_color_transition" => {
                let (from, to) = (rgb(r)?, rgb(r)?);
                Some(ParticleData::ColorScale2(from, to, r.f32()?))
            }
            "minecraft:entity_effect" => Some(ParticleData::Color(r.i32()?)),
            "minecraft:sculk_charge" => Some(ParticleData::Scalar(r.f32()?)),
            "minecraft:shriek" => Some(ParticleData::Delay(r.varint()?)),
            "minecraft:item" => match self.stack(r)? {
                Some(stack) => Some(ParticleData::Item(stack)),
                None => return Ok(None),
            },
            // PositionSource (0: a block position, 1: an entity and its y offset),
            // then the arrival time in ticks.
            "minecraft:vibration" => {
                use horizon_protocol::entity_data::PositionSource;
                let source = match r.varint()? {
                    0 => PositionSource::Block(unpack_pos(r.i64()?)),
                    1 => PositionSource::Entity {
                        id: r.varint()?,
                        y_offset: r.f32()?,
                    },
                    _ => return Ok(None),
                };
                Some(ParticleData::Vibration {
                    source,
                    ticks: r.varint()?,
                })
            }
            // A mod's: drawn by its sprites alone. Its options, when it has any, come last in
            // the packets that carry particles (entity data is framed), so they're left unread.
            n if !n.starts_with("minecraft:") => None,
            _ => None,
        };
        Ok(Some(Particle { id: *id, data }))
    }

    /// The command tree (argument properties framed by the companion mod): each
    /// parser by its client id when the client reads its properties as they
    /// are, else a string argument (greedy at the end of a command, one word
    /// before more arguments). The server still parses what the player types.
    fn commands(&self, r: &mut Reader) -> Result<p777::Commands> {
        let n = r.count(r.remaining())?;
        let mut nodes = Vec::with_capacity(n.min(1 << 14));
        for _ in 0..n {
            let flags = r.u8()?;
            let children: Vec<i32> = (0..r.count(r.remaining())?)
                .map(|_| r.varint())
                .collect::<Result<_>>()?;
            let redirect = if flags & 8 != 0 {
                Some(r.varint()?)
            } else {
                None
            };
            let kind = flags & 3;
            let name = if kind >= 1 {
                r.string(32767)?
            } else {
                String::new()
            };
            let mut parser = None;
            let mut suggestions = None;
            if kind == 2 {
                let server = r.varint()?;
                let len = r.count(r.remaining())?;
                let props = r.bytes(len)?;
                let kept = self.ctx.arguments.get(&server).copied().filter(|&id| {
                    let mut p = Reader::new(props);
                    p777::read_argument_properties(&mut p, id).is_ok() && p.remaining() == 0
                });
                parser = match (kept, self.ctx.string_argument) {
                    (Some(id), _) => Some((id, props.to_vec())),
                    (None, Some(string)) => {
                        Some((string, vec![if children.is_empty() { 2 } else { 0 }]))
                    }
                    (None, None) => None,
                };
                if flags & 16 != 0 {
                    suggestions = Some(r.string(32767)?);
                }
            }
            nodes.push(p777::CommandNode {
                kind,
                executable: flags & 4 != 0,
                restricted: false,
                children,
                redirect,
                name,
                parser,
                suggestions,
            });
        }
        Ok(p777::Commands {
            nodes,
            root: r.varint()?,
        })
    }

    /// OpenScreen: the menu by its client id, or, for a mod's menu the client
    /// lacks, a chest once its contents tell how many slots it has.
    fn open_screen(
        &mut self,
        container: i32,
        menu: i32,
        title: horizon_world::nbt::Tag,
    ) -> Vec<Vec<u8>> {
        self.generic.remove(&container);
        match self.ctx.menus.get(&menu) {
            Some(&menu) => vec![
                C777::OpenScreen(p777::OpenScreen {
                    container,
                    menu,
                    title,
                })
                .encode(),
            ],
            None => {
                self.pending_screens.insert(container, title);
                Vec::new()
            }
        }
    }

    /// A sound holder: by the client's id, or inline by name for a sound the
    /// client's registry lacks (a mod's, played from its sound files).
    #[allow(clippy::type_complexity)]
    fn sound(&self, r: &mut Reader) -> Result<Option<(i32, Option<(String, Option<f32>)>)>> {
        let id = r.varint()?;
        if id == 0 {
            let location = r.string(32767)?;
            let range = if r.bool()? { Some(r.f32()?) } else { None };
            return Ok(Some((-1, Some((location, range)))));
        }
        Ok(match self.ctx.sounds.get(&(id - 1)) {
            Some(Ok(client)) => Some((*client, None)),
            Some(Err(name)) => Some((-1, Some((name.clone(), None)))),
            None => None,
        })
    }

    /// A 767 item stack, as the companion mod frames it for the bridge: the item
    /// remapped. Its components are dropped for now (each is length-prefixed, so
    /// unknown ones skip cleanly).
    fn stack(&self, r: &mut Reader) -> Result<Option<horizon_protocol::item::Stack>> {
        let count = r.varint()?;
        if count <= 0 {
            return Ok(None);
        }
        let item = r.varint()?;
        let added = r.count(1 << 10)?;
        let removed = r.count(1 << 10)?;
        let mut stack = horizon_protocol::item::Stack::new(
            self.ctx
                .items
                .get(&item)
                .copied()
                .unwrap_or(self.ctx.placeholder_item),
            count,
        );
        for _ in 0..added {
            let kind = r.varint()?;
            let len = r.count(r.remaining())?;
            let value = r.bytes(len)?;
            if let Some(component) = self.component(kind, value) {
                stack.added.push(component);
            }
        }
        for _ in 0..removed {
            if let Some(&kind) = self.ctx.components.get(&r.varint()?) {
                stack.removed.push(kind);
            }
        }
        Ok(Some(stack))
    }

    /// A component value kept when the client has the same type and reads all of
    /// its 767 bytes as that type: a component whose encoding did not change.
    fn component(
        &self,
        kind: i32,
        value: &[u8],
    ) -> Option<(i32, horizon_protocol::item::Component)> {
        use horizon_protocol::item::Component;
        let kind = *self.ctx.components.get(&kind)?;
        let mut r = Reader::new(value);
        let component = match Component::read(kind, &mut r) {
            Ok(Component::Raw(_)) | Err(_) => return None,
            Ok(component) => component,
        };
        // 1.21.1 ends enchantments, unbreakable and dyed colour with a
        // show-in-tooltip flag that 1.21.5 moved to its own component.
        let tooltip_flag = r.remaining() == 1
            && value.last().is_some_and(|&b| b <= 1)
            && matches!(
                component,
                Component::Enchantments(_) | Component::Unit | Component::Int(_)
            );
        (r.remaining() == 0 || tooltip_flag).then_some((kind, component))
    }

    /// SetEntityData's values, 767 (framed by the companion mod) to 777: the
    /// serializers whose encoding is unchanged move to their 777 ids; block states
    /// and items are translated; the rest are left out.
    fn entity_data(&self, r: &mut Reader, w: &mut Writer) -> Result<()> {
        use horizon_protocol::entity_data::serializer as s777;
        loop {
            let index = r.u8()?;
            if index == 0xFF {
                w.u8(0xFF);
                return Ok(());
            }
            let serializer = r.varint()?;
            let len = r.count(r.remaining())?;
            let value = r.bytes(len)?;
            let same = |id: i32| Some((id, value.to_vec()));
            let out: Option<(i32, Vec<u8>)> = match serializer {
                // BYTE … DIRECTION, then OPTIONAL_UUID: unchanged ids and encodings,
                // except the item stack.
                0..=6 | 8..=13 => same(serializer),
                7 => {
                    let stack = self.stack(&mut Reader::new(value))?;
                    let mut v = Writer::new();
                    horizon_protocol::item::Slot::write(&stack, &mut v);
                    Some((s777::ITEM_STACK, v.data))
                }
                14 => {
                    let state = self.ctx.states.map(Reader::new(value).varint()?);
                    let mut v = Writer::new();
                    v.varint(state);
                    Some((s777::BLOCK_STATE, v.data))
                }
                15 => {
                    let raw = Reader::new(value).varint()?;
                    let mut v = Writer::new();
                    v.varint(if raw == 0 {
                        0
                    } else {
                        self.ctx.states.map(raw - 1) + 1
                    });
                    Some((s777::OPTIONAL_BLOCK_STATE, v.data))
                }
                19 => same(s777::VILLAGER_DATA),
                20 => same(s777::OPTIONAL_UNSIGNED_INT),
                21 => same(s777::POSE),
                23 => same(s777::WOLF_VARIANT),
                25 => same(s777::OPTIONAL_GLOBAL_POS),
                26 => same(s777::PAINTING_VARIANT),
                27 => same(s777::SNIFFER_STATE),
                28 => same(s777::ARMADILLO_STATE),
                29 => same(s777::VECTOR3),
                30 => same(s777::QUATERNION),
                _ => None,
            };
            if let Some((serializer, bytes)) = out {
                w.u8(index);
                w.varint(serializer);
                w.bytes(&bytes);
            }
        }
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
            // 767's server runs the click itself and then corrects every slot the client
            // didn't claim: sending no claims re-syncs whatever the click changed.
            S777::ContainerClick(c) => {
                let slot = match self.generic.get(&c.container) {
                    Some(g) => match g.server_slot(c.slot) {
                        Some(s) => s,
                        None => return Ok(Vec::new()),
                    },
                    None => c.slot,
                };
                vec![packet(sb::CONTAINER_CLICK, &|w| {
                    w.i8(c.container as i8);
                    w.varint(c.state);
                    w.i16(slot);
                    w.i8(c.button);
                    w.varint(c.input);
                    w.varint(0);
                    w.varint(0);
                })]
            }
            S777::ContainerClose(c) => {
                self.generic.remove(&c.container);
                vec![packet(sb::CONTAINER_CLOSE, &|w| w.u8(c.container as u8))]
            }
            S777::ContainerButtonClick(b) => vec![packet(sb::CONTAINER_BUTTON_CLICK, &|w| {
                w.i8(b.container as i8);
                w.i8(b.button as i8);
            })],
            S777::SetCreativeModeSlot(c) => {
                let item = c
                    .item
                    .as_ref()
                    .and_then(|s| Some((*self.items_back.get(&s.item)?, s.count)));
                vec![packet(sb::SET_CREATIVE_MODE_SLOT, &|w| {
                    w.i16(c.slot);
                    match item {
                        Some((item, count)) => {
                            w.varint(count);
                            w.varint(item);
                            w.varint(0);
                            w.varint(0);
                        }
                        None => w.varint(0),
                    }
                })]
            }
            // Unsigned: an offline server with secure chat off takes it as it is.
            S777::Chat(c) => vec![packet(sb::CHAT, &|w| {
                w.string(&c.message);
                w.i64(c.timestamp);
                w.i64(c.salt);
                w.bool(false);
                w.varint(c.last_seen.offset);
                w.bytes(&c.last_seen.acknowledged);
            })],
            S777::ChatCommandSigned(c) => vec![packet(sb::CHAT_COMMAND, &|w| w.string(&c.command))],
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

/// 767 light data: its four masks are `BitSet`s written as long arrays, which
/// 26.x writes as `BitSet.toByteArray` (little-endian bytes, trailing zeros
/// dropped); the sky and block layers are the same.
fn light_767(r: &mut Reader) -> Result<LightData> {
    let mask = |r: &mut Reader| -> Result<Vec<u8>> {
        let longs = r.count(r.remaining() / 8)?;
        let mut bytes = Vec::with_capacity(longs * 8);
        for _ in 0..longs {
            bytes.extend_from_slice(&r.i64()?.to_le_bytes());
        }
        let len = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
        bytes.truncate(len);
        Ok(bytes)
    };
    let (sky_mask, block_mask) = (mask(r)?, mask(r)?);
    let (empty_sky_mask, empty_block_mask) = (mask(r)?, mask(r)?);
    Ok(LightData {
        sky_mask,
        block_mask,
        empty_sky_mask,
        empty_block_mask,
        sky: horizon_protocol::chunk::Layers::read(r)?,
        block: horizon_protocol::chunk::Layers::read(r)?,
    })
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

/// ClientboundAddEntityPacket's velocity: 1/8000 of a block per tick, clamped to ±3.9.
fn velocity(v: i16) -> f64 {
    f64::from(v) / 8000.0
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
    fn mod_menus_as_chests() {
        // 5 mod slots in one row of nine, then the player's 36.
        let g = Generic {
            mod_slots: 5,
            rows: 1,
        };
        assert_eq!(g.client_slot(3), Some(3));
        assert_eq!(g.client_slot(5), Some(9));
        assert_eq!(g.server_slot(9), Some(5));
        assert_eq!(g.server_slot(7), None);
        assert_eq!(g.server_slot(-999), Some(-999));
        let items: Vec<_> = (0..41)
            .map(|i| Some(horizon_protocol::item::Stack::new(i, 1)))
            .collect();
        let shown = g.to_client(items);
        assert_eq!(shown.len(), 45);
        assert!(shown[5..9].iter().all(Option::is_none));
        assert_eq!(shown[9].as_ref().unwrap().item, 5);
    }

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
    fn light_masks_become_byte_arrays() {
        let mut w = Writer::new();
        // block mask: one long with bit 4 (0x10); the other masks empty; no layers.
        w.varint(0);
        w.varint(1);
        w.i64(0x10);
        w.varint(0);
        w.varint(0);
        w.varint(0);
        w.varint(0);
        let light = light_767(&mut Reader::new(&w.data)).unwrap();
        assert_eq!(light.block_mask, vec![0x10]);
        assert!(light.sky_mask.is_empty());
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
