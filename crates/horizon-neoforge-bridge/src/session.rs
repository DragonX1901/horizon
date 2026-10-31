//! One player's trip through the bridge: the Horizon client's connection (777)
//! on one side, the NeoForge server's (767) on the other.
use crate::facts::ServerFacts;
use crate::ids_767 as v767;
use crate::neoforge::{self, Splitter};
use crate::play::{Context, Play};
use horizon_protocol::buf::{Codec, Reader, Writer};
use horizon_protocol::connection::{ConnWriter, Connection};
use horizon_protocol::ids as v777;
use horizon_protocol::packets::configuration::ClientInformation;
use horizon_protocol::packets::{handshake, login};
use horizon_protocol::{Error, Result, text};
use horizon_world::nbt::Tag;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Where the bridge sends players and what it keeps.
#[derive(Clone, Debug)]
pub struct Options {
    pub server_host: String,
    pub server_port: u16,
    /// A folder to write each session's captured facts to (`<player>.json`).
    pub dump_dir: Option<PathBuf>,
    /// The client's resources (`facts.json`, `registries.json`): with them the
    /// bridge goes on into play; without, it stops after configuration.
    pub resources: Option<PathBuf>,
    /// Players in the world, shared by every listener of one world, and how many
    /// this listener lets in (None: no limit).
    pub players: Arc<AtomicUsize>,
    pub max_players: Option<usize>,
}

/// One counted player, uncounted when the session ends.
struct Seat(Arc<AtomicUsize>);
impl Drop for Seat {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Accept Horizon clients forever, one thread each.
pub fn serve(listener: TcpListener, options: Arc<Options>) -> std::io::Result<()> {
    for stream in listener.incoming() {
        let stream = stream?;
        let options = options.clone();
        std::thread::Builder::new()
            .name("bridge-session".into())
            .spawn(move || {
                let peer = stream
                    .peer_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_default();
                // A status ping ends with the client closing: not an error.
                if let Err(e) = handle(stream, &options)
                    && !e.is_end_of_stream()
                {
                    eprintln!("[bridge] {peer}: {e}");
                }
            })?;
    }
    Ok(())
}

fn packet(id: i32, body: impl FnOnce(&mut Writer)) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(id);
    body(&mut w);
    w.data
}

fn custom_payload_767(channel: &str, data: &[u8]) -> Vec<u8> {
    packet(v767::configuration::serverbound::CUSTOM_PAYLOAD, |w| {
        w.string(channel);
        w.bytes(data);
    })
}

fn login_disconnect(client: &mut Connection, message: &str) -> Result<()> {
    let json = serde_json::json!({ "text": message }).to_string();
    client.send(&packet(v777::login::clientbound::LOGIN_DISCONNECT, |w| {
        w.string(&json)
    }))
}

pub fn handle(stream: TcpStream, options: &Options) -> Result<()> {
    let mut client = Connection::from_stream(stream)?;
    let (id, body) = client.recv()?;
    let intention = match handshake::Serverbound::decode(id, &body)? {
        handshake::Serverbound::Intention(i) => i,
        handshake::Serverbound::Unknown(id, _) => {
            return Err(Error::new(format!("Expected a handshake, got {id}")));
        }
    };
    let mut server = Connection::connect_server(
        &options.server_host,
        options.server_port,
        Duration::from_secs(10),
    )?;
    server.send(&packet(v767::handshake::serverbound::INTENTION, |w| {
        w.varint(crate::PROTOCOL_767);
        w.string(&options.server_host);
        w.u16(options.server_port);
        // A transfer arrives at the bridge; the server sees an ordinary login.
        w.varint(if intention.intent == handshake::STATUS {
            handshake::STATUS
        } else {
            handshake::LOGIN
        });
    }))?;
    if intention.intent == handshake::STATUS {
        return status(client, server);
    }
    let _seat = Seat(options.players.clone());
    if options
        .max_players
        .is_some_and(|max| options.players.fetch_add(1, Ordering::SeqCst) >= max)
    {
        return login_disconnect(&mut client, "This world is not open to LAN.");
    }
    if options.max_players.is_none() {
        options.players.fetch_add(1, Ordering::SeqCst);
    }
    let Some(player) = login(&mut client, &mut server)? else {
        return Ok(());
    };
    let facts = configuration(client, server, &player, options)?;
    if let Some(dir) = &options.dump_dir {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{player}.json"));
        let json = serde_json::to_string_pretty(&facts.to_json())
            .map_err(|e| Error::new(e.to_string()))?;
        std::fs::write(&path, json)?;
        eprintln!("[bridge] {player}: wrote {}", path.display());
    }
    Ok(())
}

/// Status ping: forwarded both ways, with the version rewritten so Horizon lists the
/// server as joinable.
fn status(mut client: Connection, mut server: Connection) -> Result<()> {
    loop {
        let (id, body) = client.recv()?;
        server.send(&packet(id, |w| w.bytes(&body)))?;
        let (id, body) = server.recv()?;
        if id == v767::status::clientbound::STATUS_RESPONSE {
            let json = Reader::new(&body).string(32767)?;
            let mut value: serde_json::Value = serde_json::from_str(&json)
                .map_err(|e| Error::new(format!("Bad status JSON: {e}")))?;
            value["version"] = serde_json::json!({
                "name": format!("NeoForge {} (bridge)", crate::GAME_VERSION_767),
                "protocol": horizon_protocol::PROTOCOL_VERSION,
            });
            client.send(&packet(v777::status::clientbound::STATUS_RESPONSE, |w| {
                w.string(&value.to_string())
            }))?;
        } else {
            client.send(&packet(id, |w| w.bytes(&body)))?;
            return Ok(());
        }
    }
}

/// Offline login. Returns the player's name once both sides are in configuration.
fn login(client: &mut Connection, server: &mut Connection) -> Result<Option<String>> {
    let (id, body) = client.recv()?;
    let login::Serverbound::Hello(hello) = login::Serverbound::decode(id, &body)? else {
        return Err(Error::new("Expected the client's login hello"));
    };
    server.send(&packet(v767::login::serverbound::HELLO, |w| {
        w.string(&hello.name);
        w.uuid(hello.uuid);
    }))?;
    loop {
        let (id, body) = server.recv()?;
        let mut r = Reader::new(&body);
        match id {
            v767::login::clientbound::LOGIN_DISCONNECT => {
                client.send(&packet(v777::login::clientbound::LOGIN_DISCONNECT, |w| {
                    w.bytes(&body)
                }))?;
                return Ok(None);
            }
            v767::login::clientbound::HELLO => {
                login_disconnect(
                    client,
                    "This server runs in online mode. The NeoForge bridge joins offline-mode servers only \
                     for now: put the server behind the bridge with online-mode=false.",
                )?;
                return Ok(None);
            }
            v767::login::clientbound::LOGIN_COMPRESSION => server.set_compression(r.varint()?),
            v767::login::clientbound::CUSTOM_QUERY => {
                let transaction = r.varint()?;
                server.send(&packet(
                    v767::login::serverbound::CUSTOM_QUERY_ANSWER,
                    |w| {
                        w.varint(transaction);
                        w.bool(false);
                    },
                ))?;
            }
            v767::login::clientbound::COOKIE_REQUEST => {
                let key = r.string(32767)?;
                server.send(&packet(v767::login::serverbound::COOKIE_RESPONSE, |w| {
                    w.string(&key);
                    w.bool(false);
                }))?;
            }
            v767::login::clientbound::GAME_PROFILE => {
                let profile = login::GameProfile::read(&mut r)?;
                // 767 ends with strictErrorHandling, which 777 dropped.
                let _strict = r.bool()?;
                server.send(&packet(
                    v767::login::serverbound::LOGIN_ACKNOWLEDGED,
                    |_| {},
                ))?;
                let name = profile.name.clone();
                let session = profile.uuid;
                client.send(
                    &login::Clientbound::LoginFinished(login::LoginFinished { profile, session })
                        .encode(),
                )?;
                let (id, _) = client.recv()?;
                if id != v777::login::serverbound::LOGIN_ACKNOWLEDGED {
                    return Err(Error::new(format!(
                        "Expected the client's login acknowledgement, got {id}"
                    )));
                }
                return Ok(Some(name));
            }
            _ => return Err(Error::new(format!("Unexpected 767 login packet {id}"))),
        }
    }
}

/// The client's configuration packets, translated to 767. The bridge answers the
/// server's keep-alives, pings and NeoForge's tasks itself.
fn forward_client(
    mut client: horizon_protocol::connection::ConnReader,
    server: Arc<Mutex<ConnWriter>>,
    play: Receiver<Arc<Mutex<Play>>>,
) {
    let result: Result<()> = (|| {
        let send = |body: &[u8]| -> Result<()> {
            server
                .lock()
                .map_err(|_| Error::new("poisoned"))?
                .send(body)
        };
        loop {
            let (id, body) = client.recv()?;
            let mut r = Reader::new(&body);
            let out = match id {
                v777::configuration::serverbound::FINISH_CONFIGURATION => {
                    let Ok(play) = play.recv() else {
                        return Ok(());
                    };
                    send(&packet(
                        v767::configuration::serverbound::FINISH_CONFIGURATION,
                        |_| {},
                    ))?;
                    loop {
                        let (id, body) = client.recv()?;
                        let packets = play
                            .lock()
                            .map_err(|_| Error::new("poisoned"))?
                            .serverbound(id, &body)?;
                        for p in packets {
                            send(&p)?;
                        }
                    }
                }
                v777::configuration::serverbound::CLIENT_INFORMATION => {
                    let info = ClientInformation::read(&mut r)?;
                    // 767 has no particle status (added in 1.21.2).
                    packet(v767::configuration::serverbound::CLIENT_INFORMATION, |w| {
                        w.string(&info.language);
                        w.i8(info.view_distance);
                        w.varint(info.chat_visibility);
                        w.bool(info.chat_colors);
                        w.u8(info.model_customisation);
                        w.varint(info.main_hand);
                        w.bool(info.text_filtering);
                        w.bool(info.allows_listing);
                    })
                }
                v777::configuration::serverbound::CUSTOM_PAYLOAD => {
                    let channel = r.string(32767)?;
                    if channel != "minecraft:brand" {
                        continue;
                    }
                    custom_payload_767(&channel, r.rest())
                }
                _ => continue,
            };
            server
                .lock()
                .map_err(|_| Error::new("poisoned"))?
                .send(&out)?;
        }
    })();
    if let Err(e) = result
        && !e.is_end_of_stream()
    {
        eprintln!("[bridge] client: {e}");
    }
    if let Ok(server) = server.lock() {
        server.shutdown();
    }
}

enum Step {
    Continue,
    Finished,
    Disconnected(Tag),
}

struct Configuration {
    server: Arc<Mutex<ConnWriter>>,
    facts: ServerFacts,
    splitter: Splitter,
    failures: Vec<String>,
    /// 777 packets for the client, sent after each server packet.
    to_client: Vec<Vec<u8>>,
    /// Dimension types in registry order: (name, min_y, height).
    dimension_types: Vec<(String, i32, i32)>,
}

impl Configuration {
    fn send(&self, body: &[u8]) -> Result<()> {
        self.server
            .lock()
            .map_err(|_| Error::new("poisoned"))?
            .send(body)
    }
    fn reply(&self, channel: &str, data: &[u8]) -> Result<()> {
        self.send(&custom_payload_767(channel, data))
    }

    fn packet(&mut self, id: i32, body: &[u8]) -> Result<Step> {
        use v767::configuration::{clientbound as cb, serverbound as sb};
        let mut r = Reader::new(body);
        match id {
            cb::CUSTOM_PAYLOAD => {
                let channel = r.string(32767)?;
                return self.payload(&channel, r.rest());
            }
            cb::DISCONNECT => return Ok(Step::Disconnected(r.nbt()?)),
            cb::FINISH_CONFIGURATION => return Ok(Step::Finished),
            cb::KEEP_ALIVE => {
                let token = r.i64()?;
                self.send(&packet(sb::KEEP_ALIVE, |w| w.i64(token)))?;
            }
            cb::PING => {
                let token = r.i32()?;
                self.send(&packet(sb::PONG, |w| w.i32(token)))?;
            }
            cb::REGISTRY_DATA => {
                // Same wire format in 777; the client keeps entries by name.
                self.to_client.push(packet(
                    v777::configuration::clientbound::REGISTRY_DATA,
                    |w| w.bytes(body),
                ));
                let registry = r.string(32767)?;
                let mut entries = Vec::new();
                for _ in 0..r.count(1 << 20)? {
                    let name = r.string(32767)?;
                    let has_data = r.bool()?;
                    let data = if has_data { Some(r.nbt()?) } else { None };
                    if registry == "minecraft:dimension_type" {
                        let int = |k: &str| match data.as_ref().and_then(|d| d.get(k)) {
                            Some(Tag::Int(v)) => Some(*v),
                            _ => None,
                        };
                        let (min_y, height) =
                            (int("min_y").unwrap_or(-64), int("height").unwrap_or(384));
                        self.dimension_types.push((name.clone(), min_y, height));
                    }
                    entries.push((name, has_data));
                }
                self.facts.registries.insert(registry, entries);
            }
            cb::UPDATE_ENABLED_FEATURES => {
                self.to_client.push(packet(
                    v777::configuration::clientbound::UPDATE_ENABLED_FEATURES,
                    |w| w.bytes(body),
                ));
                self.facts.enabled_features = (0..r.count(1 << 10)?)
                    .map(|_| r.string(32767))
                    .collect::<Result<_>>()?;
            }
            cb::UPDATE_TAGS => {
                self.to_client
                    .push(packet(v777::configuration::clientbound::UPDATE_TAGS, |w| {
                        w.bytes(body)
                    }));
                self.facts.tag_registries = r.count(1 << 10)?;
            }
            cb::SELECT_KNOWN_PACKS => {
                let mut offered = Vec::new();
                for _ in 0..r.count(1 << 10)? {
                    offered.push((r.string(32767)?, r.string(32767)?, r.string(32767)?));
                }
                // Know no packs: every entry arrives with its data, which the
                // client needs (dimension heights) and which 26.3's packs differ from.
                let known: Vec<&(String, String, String)> = Vec::new();
                self.send(&packet(sb::SELECT_KNOWN_PACKS, |w| {
                    w.varint(known.len() as i32);
                    for (ns, id, version) in &known {
                        w.string(ns);
                        w.string(id);
                        w.string(version);
                    }
                }))?;
                self.facts.known_packs = offered;
            }
            cb::RESOURCE_PACK_PUSH => {
                let pack = r.uuid()?;
                // Accepted, then loaded: a required pack must not refuse the player.
                for action in [3, 0] {
                    self.send(&packet(sb::RESOURCE_PACK, |w| {
                        w.uuid(pack);
                        w.varint(action);
                    }))?;
                }
            }
            cb::COOKIE_REQUEST => {
                let key = r.string(32767)?;
                self.send(&packet(sb::COOKIE_RESPONSE, |w| {
                    w.string(&key);
                    w.bool(false);
                }))?;
            }
            _ => {}
        }
        Ok(Step::Continue)
    }

    fn payload(&mut self, channel: &str, data: &[u8]) -> Result<Step> {
        let mut r = Reader::new(data);
        match channel {
            "minecraft:brand" => {
                self.to_client.push(packet(
                    v777::configuration::clientbound::CUSTOM_PAYLOAD,
                    |w| {
                        w.string(channel);
                        w.bytes(data);
                    },
                ));
                self.facts.brand = Some(r.string(32767)?);
            }
            neoforge::MC_REGISTER => self.facts.listening.extend(neoforge::read_channels(data)),
            neoforge::QUERY => {
                let mut w = Writer::new();
                neoforge::write_query(&mut w, &neoforge::bridge_channels());
                self.reply(neoforge::QUERY, &w.data)?;
            }
            neoforge::NETWORK => {
                let setup = neoforge::read_setup(&mut r)?;
                if let Some(channels) = setup.get(&neoforge::Protocol::Configuration) {
                    let ids: Vec<String> = channels.keys().cloned().collect();
                    self.reply(neoforge::MC_REGISTER, &neoforge::write_channels(&ids))?;
                }
                self.facts.setup = Some(setup);
            }
            neoforge::SETUP_FAILED => {
                for _ in 0..r.count(1 << 16)? {
                    let id = r.string(32767)?;
                    let _reason = r.nbt()?;
                    self.failures.push(id);
                }
            }
            neoforge::FROZEN_SYNC_START => {
                neoforge::read_sync_start(&mut r)?;
            }
            neoforge::FROZEN_REGISTRY => {
                let (name, snapshot) = neoforge::read_frozen_registry(&mut r)?;
                self.facts.frozen.insert(name, snapshot);
            }
            neoforge::FROZEN_SYNC_COMPLETED => self.reply(neoforge::FROZEN_SYNC_COMPLETED, &[])?,
            neoforge::KNOWN_DATA_MAPS => {
                let maps = neoforge::read_known_data_maps(&mut r)?;
                let mut w = Writer::new();
                neoforge::write_known_data_maps_reply(&mut w, &maps);
                self.reply(neoforge::KNOWN_DATA_MAPS_REPLY, &w.data)?;
                self.facts.data_maps = maps;
            }
            neoforge::EXTENSIBLE_ENUM_DATA => {
                self.facts.extensible_enums = neoforge::read_extensible_enums(&mut r)?;
                self.reply(neoforge::EXTENSIBLE_ENUM_ACK, &[])?;
            }
            neoforge::FEATURE_FLAGS => {
                self.facts.modded_feature_flags = neoforge::read_feature_flags(&mut r)?;
                self.reply(neoforge::FEATURE_FLAGS_ACK, &[])?;
            }
            neoforge::CONFIG_FILE => self
                .facts
                .config_files
                .push(neoforge::read_config_file(&mut r)?.0),
            neoforge::SPLIT => {
                if let Some(whole) = self.splitter.accept(data)? {
                    let mut r = Reader::new(&whole);
                    let id = r.varint()?;
                    return self.packet(id, r.rest());
                }
            }
            neoforge::BRIDGE_ENTITY_TYPES => {
                self.facts.entity_types = neoforge::read_entity_types(&mut r)?
            }
            neoforge::BRIDGE_BLOCK_STATES => {
                self.facts.block_states = Some(neoforge::read_block_states(&mut r)?)
            }
            _ => {}
        }
        Ok(Step::Continue)
    }
}

fn configuration(
    client: Connection,
    server: Connection,
    player: &str,
    options: &Options,
) -> Result<ServerFacts> {
    let (mut server_reader, server_writer) = server.split();
    let (client_reader, mut client_writer) = client.split();
    let server_writer = Arc::new(Mutex::new(server_writer));
    let (play_tx, play_rx) = channel();
    {
        let server_writer = server_writer.clone();
        std::thread::Builder::new()
            .name("bridge-client".into())
            .spawn(move || forward_client(client_reader, server_writer, play_rx))?;
    }
    let mut state = Configuration {
        server: server_writer.clone(),
        facts: ServerFacts::default(),
        splitter: Splitter::default(),
        failures: Vec::new(),
        to_client: Vec::new(),
        dimension_types: Vec::new(),
    };
    let disconnect = |w: &mut ConnWriter, reason: &Tag| {
        w.send(&packet(v777::configuration::clientbound::DISCONNECT, |p| {
            p.nbt(reason)
        }))
    };
    loop {
        let (id, body) = server_reader.recv()?;
        let step = state.packet(id, &body)?;
        for p in state.to_client.drain(..) {
            client_writer.send(&p)?;
        }
        match step {
            Step::Continue => {}
            Step::Disconnected(reason) => {
                if !state.failures.is_empty() {
                    eprintln!(
                        "[bridge] {player}: NeoForge refused channels {:?}",
                        state.failures
                    );
                    let message = format!(
                        "The server needs mod channels the bridge did not offer ({}).\n\
                         Install the Horizon bridge companion mod on the server.",
                        state.failures.join(", ")
                    );
                    disconnect(&mut client_writer, &text::literal(&message))?;
                } else {
                    disconnect(&mut client_writer, &reason)?;
                }
                return Ok(state.facts);
            }
            Step::Finished => {
                if let Some(ctx) = options
                    .resources
                    .as_deref()
                    .and_then(|r| play_context(&state, r))
                {
                    eprintln!(
                        "[bridge] {player}: {} mod entity types announced",
                        ctx.modded_entities.len()
                    );
                    if !ctx.modded_entities.is_empty() {
                        let data = horizon_protocol::modded::write(&ctx.modded_entities);
                        client_writer.send(&packet(
                            v777::configuration::clientbound::CUSTOM_PAYLOAD,
                            |w| {
                                w.string(horizon_protocol::modded::CHANNEL);
                                w.bytes(&data);
                            },
                        ))?;
                    }
                    let play = Arc::new(Mutex::new(Play::new(ctx)));
                    let _ = play_tx.send(play.clone());
                    client_writer.send(&packet(
                        v777::configuration::clientbound::FINISH_CONFIGURATION,
                        |_| {},
                    ))?;
                    eprintln!("[bridge] {player}: configuration done, playing");
                    return play_loop(server_reader, client_writer, &play).map(|()| state.facts);
                }
                let summary = state.facts.summary();
                eprintln!("[bridge] {player}: configuration captured\n{summary}");
                let message = format!(
                    "Connected to the NeoForge server through the bridge.\n\n{summary}\n\n\
                     Playing on it is not translated yet: the bridge stops after configuration."
                );
                disconnect(&mut client_writer, &text::literal(&message))?;
                server_writer
                    .lock()
                    .map_err(|_| Error::new("poisoned"))?
                    .shutdown();
                return Ok(state.facts);
            }
        }
    }
}

/// What play translation needs: the server's block states mapped onto the
/// client's registry, and the registries' sizes and ids. None when the server
/// sent no block states (no companion mod) or the client resources don't load.
fn play_context(state: &Configuration, resources: &std::path::Path) -> Option<Context> {
    let server_states = state.facts.block_states.as_ref()?;
    let facts = std::fs::File::open(resources.join("facts.json")).ok()?;
    let (registry, flags) =
        horizon_world::registry::BlockRegistry::from_facts_json_with_flags(facts).ok()?;
    let states = crate::blocks::StateMap::build(server_states, &registry, "minecraft:stone")?;
    if !states.missing_blocks.is_empty() {
        eprintln!(
            "[bridge] {} server blocks the client lacks, drawn as stone",
            states.missing_blocks.len()
        );
    }
    let builtin = horizon_protocol::registry::Builtin::find(resources)
        .and_then(|p| horizon_protocol::registry::Builtin::load(&p).ok());
    // A frozen registry's server ids → the client's ids of the same names.
    let remap = |registry: &str| {
        let mut out = std::collections::HashMap::new();
        if let (Some(server), Some(client)) = (
            state.facts.frozen.get(registry),
            builtin.as_ref().and_then(|b| b.get(registry)),
        ) {
            for (&id, name) in &server.ids {
                if let Some(to) = client.id(name) {
                    out.insert(id, to);
                }
            }
        }
        out
    };
    let client_id = |registry: &str, name: &str| builtin.as_ref()?.get(registry)?.id(name);
    // Mod entity types the client lacks get ids past its registry, announced
    // to it (horizon_protocol::modded) before play.
    let mut entity_types = remap("minecraft:entity_type");
    let mut modded_entities = Vec::new();
    if let (Some(server), Some(client)) = (
        state.facts.frozen.get("minecraft:entity_type"),
        builtin
            .as_ref()
            .and_then(|b| b.get("minecraft:entity_type")),
    ) {
        let mut next = client.names.len() as i32;
        for (&id, name) in &server.ids {
            if entity_types.contains_key(&id) {
                continue;
            }
            if let Some((_, width, height, eye_height)) =
                state.facts.entity_types.iter().find(|t| t.0 == *name)
            {
                entity_types.insert(id, next);
                modded_entities.push(horizon_protocol::modded::ModdedEntity {
                    id: next,
                    name: name.clone(),
                    width: *width,
                    height: *height,
                    eye_height: *eye_height,
                });
                next += 1;
            }
        }
    }
    let biomes = state
        .facts
        .registries
        .get("minecraft:worldgen/biome")
        .map_or(64, Vec::len);
    Some(Context {
        client_block_bits: horizon_protocol::registry::ceil_log2(registry.len()),
        client_fluid: flags.iter().map(|&(_, fluid)| fluid).collect(),
        server_block_bits: horizon_protocol::registry::ceil_log2(server_states.state_count()),
        states,
        biomes,
        dimension_types: state.dimension_types.clone(),
        block_entity_types: remap("minecraft:block_entity_type"),
        items: remap("minecraft:item"),
        entity_types,
        modded_entities,
        menus: remap("minecraft:menu"),
        generic_menus: std::array::from_fn(|i| {
            client_id("minecraft:menu", &format!("minecraft:generic_9x{}", i + 1))
        }),
        components: remap("minecraft:data_component_type"),
        blocks: remap("minecraft:block"),
        effects: remap("minecraft:mob_effect"),
        arguments: remap("minecraft:command_argument_type"),
        string_argument: client_id("minecraft:command_argument_type", "brigadier:string"),
        attributes: remap("minecraft:attribute"),
        particles: state
            .facts
            .frozen
            .get("minecraft:particle_type")
            .map(|server| {
                server
                    .ids
                    .iter()
                    .filter_map(|(&id, name)| {
                        Some((
                            id,
                            (client_id("minecraft:particle_type", name)?, name.clone()),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        sounds: state
            .facts
            .frozen
            .get("minecraft:sound_event")
            .map(|server| {
                server
                    .ids
                    .iter()
                    .map(|(&id, name)| {
                        (
                            id,
                            client_id("minecraft:sound_event", name).ok_or_else(|| name.clone()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        placeholder_item: client_id("minecraft:item", "minecraft:barrier").unwrap_or(0),
        experience_orb: client_id("minecraft:entity_type", "minecraft:experience_orb"),
    })
}

/// Server packets to the client until either side closes.
fn play_loop(
    mut server: horizon_protocol::connection::ConnReader,
    mut client: ConnWriter,
    play: &Mutex<Play>,
) -> Result<()> {
    loop {
        let (id, body) = server.recv()?;
        let packets = play
            .lock()
            .map_err(|_| Error::new("poisoned"))?
            .clientbound(id, &body)?;
        for p in packets {
            client.send(&p)?;
        }
    }
}
